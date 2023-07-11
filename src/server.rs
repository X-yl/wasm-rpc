use std::mem::MaybeUninit;
use std::{convert::Infallible, error::Error, path::PathBuf};

use http::{Request, Response};
use http2parse::Flag;
use http2parse::Frame;
use http2parse::FrameHeader;
use http2parse::Kind;
use http2parse::Payload;
use http_body::{Body, Full};
use prost::bytes::Bytes;
use std::io::Read;
use std::io::Write;
use tonic::{body::BoxBody, server::NamedService};
use tower::Service;

pub struct Server<S> {
    path: PathBuf,
    service: S,
}

fn prepare_ack_frame(identifier: u32) -> Frame<'static> {
    let ack_payload = Payload::Settings(&[]);
    let ack_header = FrameHeader {
        length: 0,
        kind: http2parse::Kind::Settings,
        flag: Flag::ack(),
        id: http2parse::StreamIdentifier(identifier),
    };
    Frame {
        header: ack_header,
        payload: ack_payload,
    }
}

fn prepare_response_bytes<'a>(data: &'_ [u8], status: u16, target: &'a mut [u8]) -> &'a mut [u8] {
    let bytes = [
        hpack::Encoder::new().encode(
            [
                (b":status" as &[u8], format!("{}", status).as_bytes()),
                (b"content-type", b"application/grpc"),
                (b"grpc-accept-encoding", b"identity,deflate,gzip"),
            ]
            .into_iter(),
        ),
        hpack::Encoder::new().encode([(b"grpc-status" as &[u8], b"0" as &[u8])].into_iter()),
    ];
    let response_header_payload = Payload::Headers {
        priority: None,
        block: bytes[0].as_slice(),
    };
    let response_header_frame = Frame {
        header: FrameHeader {
            length: response_header_payload.encoded_len() as u32,
            kind: Kind::Headers,
            flag: Flag::end_headers(),
            id: http2parse::StreamIdentifier(1),
        },
        payload: response_header_payload,
    };

    let response_payload = Payload::Data {
        data: data.as_ref(),
    };
    let response_frame = Frame {
        header: FrameHeader {
            length: response_payload.encoded_len() as u32,
            kind: Kind::Data,
            flag: Flag::empty(),
            id: http2parse::StreamIdentifier(1),
        },
        payload: response_payload,
    };
    let response_header_payload_2 = Payload::Headers {
        priority: None,
        block: bytes[1].as_slice(),
    };
    let response_header_frame_2 = Frame {
        header: FrameHeader {
            length: response_header_payload_2.encoded_len() as u32,
            kind: Kind::Headers,
            flag: Flag::end_headers() | Flag::end_stream(),
            id: http2parse::StreamIdentifier(1),
        },
        payload: response_header_payload_2,
    };

    let mut n = 0;
    n += response_header_frame.encode(&mut target[n..]);
    n += response_frame.encode(&mut target[n..]);
    n += response_header_frame_2.encode(&mut target[n..]);
    &mut target[..n]
}

pub fn prepare_initial_settings() -> [Frame<'static>; 2] {
    let payload1 = Payload::Settings(&[]);
    let settings = Frame {
        header: FrameHeader {
            length: payload1.encoded_len() as u32,
            kind: Kind::Settings,
            flag: Flag::empty(),
            id: http2parse::StreamIdentifier(0),
        },
        payload: payload1,
    };

    let payload2 = Payload::WindowUpdate(http2parse::SizeIncrement(2u32.pow(31) - 1));
    let window_update = Frame {
        header: FrameHeader {
            length: payload2.encoded_len() as u32,
            kind: Kind::WindowUpdate,
            flag: Flag::empty(),
            id: http2parse::StreamIdentifier(0),
        },
        payload: payload2,
    };

    [settings, window_update]
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

    pub async fn serve(mut self) -> Result<(), Box<dyn Error>> {
        let socket_server = std::os::unix::net::UnixListener::bind(self.path).unwrap();

        loop {
            match socket_server.accept() {
                Ok((mut stream, _)) => {
                    let buf = &mut [0; 512];
                    let n = stream.read(buf)?;
                    const PREFACE: &[u8; 24] = b"PRI * HTTP/2.0\r\n\r\nSM\r\n\r\n";
                    assert!(&buf[..n].starts_with(PREFACE));

                    let mut pos = 24;

                    while pos < n {
                        let header = FrameHeader::parse(&buf[pos..pos + 9]).unwrap();
                        let _frame =
                            Frame::parse(header, &buf[pos + 9..pos + 9 + header.length as usize])
                                .unwrap();

                        pos += 9 + header.length as usize;
                    }

                    let mut n = 0;
                    for frame in prepare_initial_settings() {
                        n += frame.encode(&mut buf[n..]);
                    }
                    stream.write_all(&buf[..n]).unwrap();

                    let n = prepare_ack_frame(0).encode(buf);
                    stream.write_all(&buf[..n]).unwrap();
                    let n = stream.read(buf)?;

                    let mut pos = 0;
                    let mut headers = None;
                    let mut data = None;

                    while pos < n {
                        let header = FrameHeader::parse(&buf[pos..pos + 9]).unwrap();
                        let frame: Frame =
                            Frame::parse(header, &buf[pos + 9..pos + 9 + header.length as usize])
                                .unwrap();
                        println!("{:?}", frame);

                        if frame.header.kind == Kind::Headers {
                            headers.replace(match frame.payload {
                                Payload::Headers { block, .. } => {
                                    hpack::Decoder::new().decode(block).unwrap()
                                }
                                _ => unreachable!(),
                            });
                        } else if frame.header.kind == Kind::Data {
                            data.replace(match frame.payload {
                                Payload::Data { data } => data.clone(),
                                _ => unreachable!(),
                            });
                        }
                        pos += 9 + header.length as usize;
                    }
                    let headers = headers.unwrap();
                    let uri = &headers
                        .iter()
                        .find(|(k, _)| k.as_slice() == b":path")
                        .unwrap()
                        .1;

                    let http_req = http::Request::options(uri.as_slice())
                        .body(Full::new(Bytes::copy_from_slice(data.unwrap())))?;

                    let resp = self.service.call(http_req).await?;
                    let status = resp.status().as_u16();

                    let data = resp.into_body().data().await.unwrap().unwrap();

                    stream
                        .write_all(prepare_response_bytes(data.as_ref(), status, buf))
                        .unwrap();
                }
                Err(err) => {
                    eprintln!("err: {:?}", err);
                }
            }
        }
    }
}
