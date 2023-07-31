use http2parse::StreamIdentifier;
use http_body::combinators::UnsyncBoxBody;
use std::collections::HashMap;
use std::pin::Pin;
use std::task::{Context, Poll};
use std::{convert::Infallible, error::Error, path::PathBuf};
use tokio::io::AsyncWriteExt;
use tokio::sync::mpsc::{self, Receiver, Sender};
use tokio::sync::oneshot;
use tonic::Status;

use http::{Request, Response};
use http2parse::Flag;
use http2parse::Frame;
use http2parse::FrameHeader;
use http2parse::Kind;
use http2parse::Payload;
use http_body::{Body, Full};
use ouroboros::self_referencing;
use prost::bytes::Bytes;
use tonic::codegen::futures_core::Stream;
use tonic::{body::BoxBody, server::NamedService};
use tower::Service;

use crate::http2::PREFACE;

pub struct Server<S> {
    path: PathBuf,
    service: S,
}

fn prepare_ack_frame(identifier: u32) -> Frame<'static> {
    let ack_payload = Payload::Settings(&[]);
    let ack_header = FrameHeader {
        length: ack_payload.encoded_len() as u32,
        kind: http2parse::Kind::Settings,
        flag: Flag::ack(),
        id: http2parse::StreamIdentifier(identifier),
    };
    Frame {
        header: ack_header,
        payload: ack_payload,
    }
}

fn prepare_response_header(stream_id: StreamIdentifier, status: u16) -> OwnedFrame {
    let bytes = //[
        hpack::Encoder::new().encode(
            [
                (b":status" as &[u8], format!("{}", status).as_bytes()),
                (b"content-type", b"application/grpc"),
                (b"grpc-accept-encoding", b"identity,deflate,gzip"),
            ]
            .into_iter(),
        );

    return OwnedFrameBuilder {
        buf: bytes.into_boxed_slice(),
        frame_builder: |bytes| {
            let response_header_payload = Payload::Headers {
                priority: None,
                block: bytes,
            };

            Frame {
                header: FrameHeader {
                    length: response_header_payload.encoded_len() as u32,
                    kind: Kind::Headers,
                    flag: Flag::end_headers(),
                    id: stream_id,
                },
                payload: response_header_payload,
            }
        },
    }
    .build();
}
fn prepare_response_trailers(stream_id: StreamIdentifier) -> OwnedFrame {
    let bytes =
        hpack::Encoder::new().encode([(b"grpc-status" as &[u8], b"0" as &[u8])].into_iter());

    return OwnedFrameBuilder {
        buf: bytes.into_boxed_slice(),
        frame_builder: |bytes| {
            let response_header_payload = Payload::Headers {
                priority: None,
                block: bytes,
            };

            Frame {
                header: FrameHeader {
                    length: response_header_payload.encoded_len() as u32,
                    kind: Kind::Headers,
                    flag: Flag::end_headers() | Flag::end_stream(),
                    id: stream_id,
                },
                payload: response_header_payload,
            }
        },
    }
    .build();
}

fn prepare_response(stream_id: StreamIdentifier, data: &[u8]) -> OwnedFrame {
    OwnedFrameBuilder {
        buf: Vec::from(data).into_boxed_slice(),
        frame_builder: |buf| {
            let response_payload = Payload::Data { data: buf.as_ref() };
            Frame {
                header: FrameHeader {
                    length: response_payload.encoded_len() as u32,
                    kind: Kind::Data,
                    flag: Flag::empty(),
                    id: stream_id,
                },
                payload: response_payload,
            }
        },
    }
    .build()
}

#[self_referencing]
#[derive(Debug)]
struct OwnedFrame {
    buf: Box<[u8]>,
    #[borrows(buf)]
    #[covariant]
    frame: Frame<'this>,
}

impl<S> Server<S>
where
    S: for<'a> Service<Request<Full<Bytes>>, Response = Response<BoxBody>, Error = Infallible>
        + NamedService
        + Clone
        + Send
        + 'static,
{
    pub fn new(path: PathBuf, service: S) -> Self {
        Server { path, service }
    }

    async fn handle_connection(
        stream_id: StreamIdentifier,
        mut rx: Receiver<OwnedFrame>,
        tx_frame: Sender<OwnedFrame>,
        tx_request: Sender<(
            http::Request<Full<Bytes>>,
            oneshot::Sender<http::Response<UnsyncBoxBody<Bytes, Status>>>,
        )>,
    ) {
        if stream_id.0 == 0 {
            let ack = prepare_ack_frame(0);
            tx_frame
                .send(
                    OwnedFrameBuilder {
                        buf: Box::new([]),
                        frame_builder: |_| ack,
                    }
                    .build(),
                )
                .await
                .unwrap();
            let settings = crate::http2::prepare_initial_settings();
            for s in settings {
                tx_frame
                    .send(
                        OwnedFrameBuilder {
                            buf: Box::new([]),
                            frame_builder: |_| s,
                        }
                        .build(),
                    )
                    .await
                    .unwrap();
            }
        }
        let mut headers = None;

        loop {
            let owned_frame = rx.recv().await.unwrap();
            let frame = owned_frame.borrow_frame();

            match frame.payload {
                Payload::Headers { block, .. } => {
                    headers = Some(hpack::Decoder::new().decode(block).unwrap())
                }
                Payload::Data { data } => {
                    let headers = headers.as_ref().unwrap();
                    let uri = &headers
                        .iter()
                        .find(|(k, _)| k.as_slice() == b":path")
                        .unwrap()
                        .1;

                    let http_req = http::Request::options(uri.as_slice())
                        .body(Full::new(Bytes::copy_from_slice(data)))
                        .unwrap();

                    let (tx, rx) = oneshot::channel();
                    tx_request.send((http_req, tx)).await.unwrap();

                    let resp = rx.await.unwrap();
                    let status = resp.status().as_u16();
                    let mut body = resp.into_body();
                    tx_frame
                        .send(prepare_response_header(stream_id, status))
                        .await
                        .unwrap();

                    while let Some(Ok(chunk)) = body.data().await {
                        println!("resp: {:?}", String::from_utf8_lossy(&chunk));
                        tx_frame
                            .send(prepare_response(stream_id, chunk.as_ref()))
                            .await
                            .unwrap();
                    }

                    tx_frame
                        .send(prepare_response_trailers(stream_id))
                        .await
                        .unwrap();
                }
                Payload::Ping(x) => {
                    let payload = Payload::Ping(x);
                    let frame = Frame {
                        header: FrameHeader {
                            length: payload.encoded_len() as u32,
                            kind: Kind::Ping,
                            flag: Flag::empty(),
                            id: stream_id,
                        },
                        payload,
                    };

                    tx_frame
                        .send(
                            OwnedFrameBuilder {
                                buf: Box::new([]),
                                frame_builder: |_| frame,
                            }
                            .build(),
                        )
                        .await
                        .unwrap();
                }
                _ => {
                    println!("ignored frame {:?}", frame.payload);
                }
            }
        }
    }

    pub async fn serve(mut self) -> Result<(), Box<dyn Error>>
    where
        <S as Service<http::Request<http_body::Full<prost::bytes::Bytes>>>>::Future: Send,
    {
        let socket_server = tokio::net::UnixListener::bind(self.path).unwrap();
        let (service_tx, mut service_rx) = mpsc::channel(32);
        tokio::spawn(async move {
            loop {
                let (req, resp_channel): (
                    Request<Full<Bytes>>,
                    oneshot::Sender<Response<UnsyncBoxBody<Bytes, Status>>>,
                ) = service_rx.recv().await.unwrap();
                let resp = self.service.call(req).await.unwrap();
                resp_channel.send(resp).unwrap();
            }
        });

        loop {
            match socket_server.accept().await {
                Ok((stream, _)) => {
                    let service_tx = service_tx.clone();
                    tokio::spawn(async move {
                        let mut tasks = Vec::new();
                        let (tx_to_self, mut self_rx) = mpsc::channel(32);
                        let (read_stream, mut write_stream) = stream.into_split();
                        tasks.push(tokio::spawn(async move {
                            loop {
                                let resp: OwnedFrame = self_rx.recv().await.unwrap();
                                let mut buf = [0u8; 512];
                                let n = resp.borrow_frame().encode(&mut buf);
                                write_stream.write_all(&buf[..n]).await.unwrap();
                            }
                        }));
                        let mut streams = HashMap::new();
                        loop {
                            let mut buf = [0; 512];
                            read_stream.readable().await.unwrap();
                            let n = match read_stream.try_read(&mut buf) {
                                Ok(x) => x,
                                Err(ref e) if e.kind() == std::io::ErrorKind::WouldBlock => {
                                    continue
                                }
                                Err(e) => {
                                    println!("done");
                                    return Err::<(), Box<dyn Error + Send>>(Box::new(e));
                                }
                            };
                            // println!("{:?}", n);
                            if n == 0 {
                                break;
                            }
                            let mut frames = Vec::new();
                            let mut pos = if buf[..n].starts_with(PREFACE) { 24 } else { 0 };
                            while pos < n {
                                let header = FrameHeader::parse(&buf[pos..pos + 9]).unwrap();
                                let owned_frame = OwnedFrameBuilder {
                                    buf: Vec::from(&buf[pos + 9..pos + 9 + header.length as usize])
                                        .into_boxed_slice(),
                                    frame_builder: |buf| {
                                        Frame::parse(header, buf.as_ref()).unwrap()
                                    },
                                }
                                .build();
                                frames.push(owned_frame);
                                pos += 9 + header.length as usize;
                            }
                            for frame in frames {
                                let stream_id = frame.borrow_frame().header.id.0;
                                let tx_to_handler = match streams.get(&stream_id) {
                                    Some(x) => x,
                                    None => {
                                        let (tx_to_handler, handler_rx) = mpsc::channel(32);
                                        tasks.push(tokio::spawn(Self::handle_connection(
                                            frame.borrow_frame().header.id,
                                            handler_rx,
                                            tx_to_self.clone(),
                                            service_tx.clone(),
                                        )));
                                        streams.insert(stream_id, tx_to_handler);
                                        streams.get(&stream_id).unwrap()
                                    }
                                };
                                tx_to_handler.send(frame).await.unwrap();
                            }
                        }
                        for task in tasks {
                            task.abort();
                        }
                        Ok(())
                    });
                }
                Err(err) => {
                    eprintln!("Failed to accept socket connection: {:?}", err);
                }
            }
        }
    }
}

pub struct ReceiverStream<T> {
    inner: Receiver<T>,
}

impl<T> ReceiverStream<T> {
    /// Create a new `ReceiverStream`.
    pub fn new(recv: Receiver<T>) -> Self {
        Self { inner: recv }
    }

    /// Get back the inner `Receiver`.
    pub fn into_inner(self) -> Receiver<T> {
        self.inner
    }
}

impl<T> Stream for ReceiverStream<T> {
    type Item = T;

    fn poll_next(mut self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Option<Self::Item>> {
        self.inner.poll_recv(cx)
    }
}
