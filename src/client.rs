use std::{
    future::Future,
    path::PathBuf,
    pin::Pin,
    sync::{
        atomic::{AtomicBool, AtomicU32},
        mpsc,
    },
    task::Poll,
};

use anyhow::Result;
use http::{Request, Response};
use http2parse::{Flag, Frame, FrameHeader, Kind, Payload};
use http_body::combinators::UnsyncBoxBody;
use prost::bytes::Bytes;
use tonic::{body::BoxBody, codegen::Body};
use tower::Service;
use wasm_rs_async_executor::single_threaded as executor;

use crate::http2::PREFACE;

static CONNECTION_INITIALIZED: AtomicBool = AtomicBool::new(false);
static STREAM_ID: AtomicU32 = AtomicU32::new(1);

#[derive(Debug, Clone)]
pub struct Client {
    path: PathBuf,
}

impl Client {
    pub fn new(path: PathBuf) -> Self {
        Client { path }
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
        Box::pin(call(self.path.clone(), req))
    }
}

fn prepare_request_bytes<'a>(uri: &'_ [u8], data: &'_ [u8], target: &'a mut [u8]) -> &'a mut [u8] {
    let mut n = 0;
    if !CONNECTION_INITIALIZED.swap(true, std::sync::atomic::Ordering::Relaxed) {
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
        n += settings.encode(&mut target[n..]);
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

    let stream_id = STREAM_ID.fetch_add(2, std::sync::atomic::Ordering::Relaxed);
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

    n += header_frame.encode(&mut target[n..]);
    n += data_frame.encode(&mut target[n..]);

    &mut target[..n]
}

pub async fn call(
    path: PathBuf,
    request: Request<BoxBody>,
) -> Result<Response<UnsyncBoxBody<Bytes, anyhow::Error>>> {
    let uri = request.uri().to_string();
    let data = request
        .into_body()
        .data()
        .await
        .ok_or(anyhow::anyhow!("No available data."))??;

    #[cfg(target_family = "wasm")]
    {
        let mut buf = [0u8; 512];
        buf[..PREFACE.len()].copy_from_slice(PREFACE);
        let mut n = PREFACE.len();
        if !CONNECTION_INITIALIZED.load(std::sync::atomic::Ordering::Relaxed) {
            for frame in crate::http2::prepare_initial_settings() {
                n += frame.encode(&mut buf[n..]);
            }
            while let Err(_) = std::fs::write(&path, &buf[..n]) {
                continue;
            }
        }

        std::fs::write(
            &path,
            prepare_request_bytes(&uri.as_bytes(), data.as_ref(), &mut buf),
        )?;

        let mut pos = 0;

        let (tx, rx) = mpsc::channel();
        let http_resp = http::Response::builder()
            .status(200)
            .body(StreamingBody { rx }.boxed_unsync())?;

        executor::spawn(async move {
            let exec = async {
                let mut resp = std::fs::read(&path)?;
                'outer: loop {
                    resp.extend(std::fs::read(&path)?);
                    while pos < resp.len() {
                        let header = FrameHeader::parse(match &resp.get(pos..pos + 9) {
                            Some(x) => x,
                            None => break,
                        })?;

                        let frame = Frame::parse(
                            header,
                            match &resp.get(pos + 9..pos + 9 + header.length as usize) {
                                Some(x) => x,
                                None => break,
                            },
                        )?;

                        match frame.payload {
                            Payload::Data { data } => {
                                if let Err(_) = tx.send(Ok(Bytes::copy_from_slice(data))) {
                                    break;
                                }
                            }
                            Payload::Headers { .. } => {
                                if frame.header.flag.contains(Flag::end_stream()) {
                                    break 'outer Ok::<(), anyhow::Error>(());
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
                                let mut buf2 = [0u8; 512];
                                let n = frame.encode(&mut buf2);
                                std::fs::write(&path, &buf2[..n])?;
                            }
                            _ => {}
                        }

                        pos += 9 + header.length as usize;
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
