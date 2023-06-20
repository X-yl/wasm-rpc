use std::{convert::Infallible, error::Error, path::PathBuf};

use http::{Request, Response};
use http_body::{Body, Full};
use prost::bytes::Bytes;
use std::io::Read;
use std::io::Write;
use tonic::{body::BoxBody, server::NamedService};
use tower::Service;

use crate::transfer;

pub struct Server<S> {
    path: PathBuf,
    service: S,
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
                    // FIXME: Why does this need to live for 'static?
                    let buf = Box::leak(Box::new([0; 512]));
                    let n = stream.read(buf)?;
                    if let Ok((req, _)) = postcard::take_from_bytes::<transfer::Request>(&buf[..n])
                    {
                        let http_req = http::Request::options(req.uri)
                            .body(Full::new(Bytes::from_static(req.body)))?;

                let resp = self.service.call(http_req).await?;
        
                        let status = resp.status().as_u16();
                        let data = resp.into_body().data().await.unwrap().unwrap();

                        let transfer = transfer::Response {
                            status,
                            body: &data,
                        };
                        let serialized =
                            postcard::to_vec::<transfer::Response, 512>(&transfer).unwrap();

                stream.write_all(serialized.as_slice())?;
                    }
                }
                Err(err) => {
                    eprintln!("err: {:?}", err);
                }
            }
        }        
    }
}
