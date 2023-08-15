#[cfg(unix)]
use std::os::unix::net::UnixStream;
use std::{
    future::Future,
    io::{ErrorKind, Write},
    path::{Path, PathBuf},
    pin::Pin,
    sync::{
        atomic::{AtomicBool, AtomicU32},
        mpsc::{self, Sender},
        Arc, Mutex,
    },
    task::Poll,
    time::Duration,
};

use anyhow::Result;
use http::{Request, Response};
use http2parse::{Flag, Frame, FrameHeader, Kind, Payload, StreamIdentifier};
use http_body::combinators::UnsyncBoxBody;
use prost::bytes::Bytes;
use tonic::{body::BoxBody, codegen::Body};
use tower::Service;
use wasm_rs_async_executor::single_threaded as executor;

use crate::http2::PREFACE;

// Dummy so that we can use Option<UnixStream>
#[cfg(target_arch = "wasm32")]
struct UnixStream;

#[derive(Debug, Clone)]
pub struct Client {
    path: PathBuf,
    stream_id: Arc<AtomicU32>,
    connection_initalized: Arc<AtomicBool>,
}

impl Client {
    pub fn new(path: PathBuf) -> Self {
        Client {
            path,
            stream_id: Arc::new(0.into()),
            connection_initalized: Arc::new(false.into()),
        }
    }
}

impl Service<Request<BoxBody>> for Client {
    type Response = Response<UnsyncBoxBody<Bytes, anyhow::Error>>;
    type Error = anyhow::Error;
    type Future = Pin<Box<dyn Future<Output = Result<Self::Response, Self::Error>>>>;

    fn poll_ready(
        &mut self,
        _cx: &mut std::task::Context<'_>,
    ) -> std::task::Poll<Result<(), Self::Error>> {
        Poll::Ready(Ok(()))
    }

    fn call(&mut self, req: Request<BoxBody>) -> Self::Future {
        Box::pin(call(
            self.path.clone(),
            self.connection_initalized.clone(),
            self.stream_id.clone(),
            req,
        ))
    }
}

fn prepare_request_bytes<'a>(
    connection_initialized: Arc<AtomicBool>,
    stream_id: Arc<AtomicU32>,
    uri: &'_ [u8],
    data: &'_ [u8],
) -> (Vec<u8>, StreamIdentifier) {
    let mut n = 0;
    let mut buf = vec![];
    if !connection_initialized.swap(true, std::sync::atomic::Ordering::Relaxed) {
        let payload1 = Payload::Settings(&[]);
        let settings = Frame {
            header: FrameHeader {
                length: payload1.encoded_len() as u32,
                kind: Kind::Settings,
                flag: Flag::ack(),
                id: http2parse::StreamIdentifier(0),
            },
            payload: payload1,
        };
        buf.resize(buf.len() + settings.encoded_len(), 0);
        n += settings.encode(&mut buf[n..]);
    }

    let header_bytes = hpack::Encoder::new().encode(
        [
            (b":path" as &[u8], uri),
            (b":method", b"POST"),
            (b"content-type", b"application/grpc"),
            (b"grpc-accept-encoding", b"identity,deflate,gzip"),
            (b"te", b"trailers"),
            (b":scheme", b"http"),
            (b":authority", b"localhost:1443"),
        ]
        .into_iter(),
    );

    let stream_id = stream_id.fetch_add(2, std::sync::atomic::Ordering::Relaxed);
    let header_payload = Payload::Headers {
        priority: None,
        block: header_bytes.as_slice(),
    };
    let header_frame = Frame {
        header: FrameHeader {
            length: header_payload.encoded_len() as u32,
            kind: Kind::Headers,
            flag: Flag::end_headers(),
            id: http2parse::StreamIdentifier(stream_id),
        },
        payload: header_payload,
    };

    let data_payload = Payload::Data { data };
    let data_frame = Frame {
        header: FrameHeader {
            length: data_payload.encoded_len() as u32,
            kind: Kind::Data,
            flag: Flag::empty(),
            id: http2parse::StreamIdentifier(stream_id),
        },
        payload: data_payload,
    };

    buf.resize(
        buf.len() + header_frame.encoded_len() + data_frame.encoded_len(),
        0,
    );
    n += header_frame.encode(&mut buf[n..]);
    data_frame.encode(&mut buf[n..]);

    (buf, StreamIdentifier(stream_id))
}

pub async fn call(
    path: PathBuf,
    connection_initialized: Arc<AtomicBool>,
    stream_id: Arc<AtomicU32>,
    request: Request<BoxBody>,
) -> Result<Response<UnsyncBoxBody<Bytes, anyhow::Error>>> {
    let uri = request.uri().to_string();
    let path_copy = path.clone();

    let initial_frames = crate::http2::prepare_initial_settings();
    let mut buf = vec![
        0;
        PREFACE.len()
            + initial_frames
                .iter()
                .map(|f| f.encoded_len())
                .sum::<usize>()
    ];
    buf[..PREFACE.len()].copy_from_slice(PREFACE);
    let mut n = PREFACE.len();
    let mut stream: Option<UnixStream> = None;

    if !connection_initialized.load(std::sync::atomic::Ordering::Relaxed) {
        for frame in initial_frames {
            n += frame.encode(&mut buf[n..]);
        }
        #[cfg(unix)]
        {
            stream.replace(loop {
                match std::os::unix::net::UnixStream::connect(&path) {
                    Ok(x) => break x,
                    Err(_) => continue,
                }
            });
            stream.as_mut().unwrap().set_nonblocking(true)?;
            stream.as_mut().unwrap().write_all(&buf[..n])?;
        }

        #[cfg(target_arch = "wasm32")]
        while let Err(e) = std::fs::write(&path, &buf[..n]) {
            continue;
        }
    }
    let mut pos = 0;

    let (tx, rx) = mpsc::channel();
    let tx_copy = tx.clone();

    let stream = Arc::new(Mutex::new(stream));
    let stream1 = stream.clone();

    executor::spawn(async move {
        let exec = async {
            let mut body = request.into_body();
            let data = body
                .data()
                .await
                .ok_or(anyhow::anyhow!("No available data."))??;
            let (bytes, stream_id) = prepare_request_bytes(
                connection_initialized,
                stream_id,
                &uri.as_bytes(),
                data.as_ref(),
            );
            #[cfg(target_arch = "wasm32")]
            std::fs::write(&path_copy, bytes)?;
            #[cfg(unix)]
            {
                stream1
                    .lock()
                    .as_mut()
                    .unwrap()
                    .as_mut()
                    .unwrap()
                    .write_all(&bytes)?;
            }

            while let Some(Ok(data)) = body.data().await {
                let data_payload = Payload::Data { data: &data };
                let data_frame = Frame {
                    header: FrameHeader {
                        length: data_payload.encoded_len() as u32,
                        kind: Kind::Data,
                        flag: Flag::empty(),
                        id: stream_id,
                    },
                    payload: data_payload,
                };
                let mut buf = vec![0u8; data_frame.encoded_len()];
                let n = data_frame.encode(&mut buf);
                #[cfg(target_arch = "wasm32")]
                std::fs::write(&path_copy, &buf[..n])?;
                #[cfg(unix)]
                stream1
                    .lock()
                    .as_mut()
                    .unwrap()
                    .as_mut()
                    .unwrap()
                    .write_all(&buf[..n])?;
            }

            Ok::<(), anyhow::Error>(())
        };
        if let Err(e) = exec.await {
            let _ = tx_copy.send(Err(e));
        }
    });

    let http_resp = http::Response::builder()
        .status(200)
        .body(StreamingBody { rx }.boxed_unsync())?;

    executor::spawn(async move {
        let exec = async {
            let mut n = 0;
            #[cfg(target_arch = "wasm32")]
            let mut resp = {
                let r = std::fs::read(&path)?;
                n += r.len();
                r
            };
            #[cfg(unix)]
            let mut resp = vec![];
            'outer: loop {
                #[cfg(target_arch = "wasm32")]
                {
                    let r = std::fs::read(&path)?;
                    n += r.len();
                    resp.extend(r);
                }
                #[cfg(unix)]
                {
                    use std::io::Read;
                    if resp.len() < n + 10240 {
                        resp.resize(n + 10240, 0);
                    }
                    n += match stream
                        .lock()
                        .as_mut()
                        .unwrap()
                        .as_mut()
                        .unwrap()
                        .read(&mut resp[n..])
                    {
                        Err(e) if e.kind() == ErrorKind::WouldBlock => 0,
                        Ok(x) => {
                            if x == 0 {
                                return Ok(());
                            } else {
                                x
                            }
                        }
                        Err(e) => return Err(anyhow::anyhow!(e)),
                    };
                }
                while pos < n {
                    let header = FrameHeader::parse(match &resp[..n].get(pos..(pos + 9)) {
                        Some(x) => x,
                        None => break,
                    })?;

                    let frame = Frame::parse(
                        header,
                        match &resp[..n].get((pos + 9)..(pos + 9 + header.length as usize)) {
                            Some(x) => x,
                            None => break,
                        },
                    )?;
                    if handle_payload(&frame, tx.clone(), &path, stream.clone())? {
                        break 'outer Ok::<(), anyhow::Error>(());
                    }
                    pos += 9 + header.length as usize;
                    resp.drain(0..pos);
                    n -= pos;
                    pos = 0;
                }
                YieldNow { yielded: false }.await
            }
        };
        if let Err(e) = exec.await {
            let _ = tx.send(Err(e));
        }
    });
    Ok(http_resp)
}

struct StreamingBody {
    rx: mpsc::Receiver<Result<Bytes>>,
}

struct YieldNow {
    yielded: bool,
}

impl Future for YieldNow {
    type Output = ();

    fn poll(mut self: Pin<&mut Self>, _cx: &mut std::task::Context<'_>) -> Poll<Self::Output> {
        if !self.yielded {
            self.as_mut().yielded = true;
            return Poll::Pending;
        } else {
            return Poll::Ready(());
        }
    }
}

impl Body for StreamingBody {
    type Data = Bytes;

    type Error = anyhow::Error;

    fn poll_data(
        self: Pin<&mut Self>,
        _cx: &mut std::task::Context<'_>,
    ) -> Poll<Option<Result<Self::Data, Self::Error>>> {
        match self.rx.try_recv() {
            Ok(x) => Poll::Ready(Some(x)),
            Err(mpsc::TryRecvError::Empty) => Poll::Pending,
            Err(mpsc::TryRecvError::Disconnected) => Poll::Ready(None),
        }
    }

    fn poll_trailers(
        self: Pin<&mut Self>,
        _cx: &mut std::task::Context<'_>,
    ) -> Poll<Result<Option<http::HeaderMap>, Self::Error>> {
        Poll::Ready(Ok(None))
    }
}

fn handle_payload(
    frame: &Frame,
    tx: Sender<Result<Bytes>>,
    path: &Path,
    stream: Arc<Mutex<Option<UnixStream>>>,
) -> Result<bool> {
    match frame.payload {
        Payload::Data { data } => if let Err(_) = tx.send(Ok(Bytes::copy_from_slice(data))) {},
        Payload::Headers { .. } => {
            if frame.header.flag.contains(Flag::end_stream()) {
                return Ok(true);
            }
        }
        Payload::Ping(i) => {
            let payload = Payload::Ping(i);
            let frame = Frame {
                header: FrameHeader {
                    length: payload.encoded_len() as u32,
                    kind: Kind::Ping,
                    flag: Flag::ack(),
                    id: http2parse::StreamIdentifier(1),
                },
                payload,
            };
            let mut buf2 = vec![0; frame.encoded_len()];
            let n = frame.encode(&mut buf2);
            #[cfg(target_arch = "wasm32")]
            std::fs::write(&path, &buf2[..n])?;
            #[cfg(unix)]
            stream
                .lock()
                .as_mut()
                .unwrap()
                .as_mut()
                .unwrap()
                .write_all(&buf2[..n])?;
        }
        _ => {}
    }
    Ok(false)
}
