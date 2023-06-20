use std::{future::Future, path::PathBuf, pin::Pin, task::Poll, convert::Infallible};

use http::{Request, Response};
use http_body::{Full, combinators::UnsyncBoxBody};
use prost::bytes::Bytes;
use tonic::{body::BoxBody, codegen::Body};
use tower::Service;

use crate::transfer;

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
    type Response = Response<UnsyncBoxBody<Bytes, Infallible>>;
    type Error = std::io::Error;
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

pub async fn call(
    path: PathBuf,
    request: Request<BoxBody>,
) -> Result<Response<UnsyncBoxBody<Bytes, Infallible>>, std::io::Error> {
    let uri = request.uri().to_string();
    let data = request.into_body().data().await.unwrap().unwrap();
    let transfer = transfer::Request { uri, body: &data };
    let serialized = postcard::to_vec::<transfer::Request, 512>(&transfer).unwrap();

    #[cfg(target_family = "wasm")]
    {
        std::fs::write(&path, serialized.as_slice())?;
        // FIXME: why is this 'static
        let resp = Vec::leak(std::fs::read(&path)?);

        if let Ok(resp) = postcard::from_bytes::<transfer::Response>(resp) {
            let http_resp = http::Response::builder()
                .status(resp.status)
                .body(Full::new(Bytes::from_static(resp.body)).boxed_unsync()).unwrap();

            return Ok(http_resp)
        } else {
            return Err(std::io::Error::new(std::io::ErrorKind::AddrNotAvailable, "oh no"))
        }
    }

    #[cfg(unix)]
    {
        use std::io::Write;
        use std::os::unix::net::UnixStream;
        let mut stream = UnixStream::connect(path)?;
        stream.write_all(serialized.as_slice())?;
        let buf = Box::leak(Box::new([0; 512]));
        let n = stream.read(buf)?;
        if let Ok((resp, _)) = postcard::take_from_bytes::<transfer::Response>(&buf[..n]) {
            println!("valid resp {:?}", resp);
            let http_resp = http::Response::builder()
                .status(resp.status)
                .body(Full::new(Bytes::from_static(resp.body)).boxed_unsync()).unwrap();

            return Ok(http_resp)
        }
    }
}
