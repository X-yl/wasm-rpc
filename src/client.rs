use std::{path::PathBuf, pin::Pin, future::Future, task::Poll};

use http::{Request, Response};
use tonic::{body::BoxBody, codegen::Body};
use tower::Service;


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
    type Response = Response<String>;
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

pub async fn call(path: PathBuf, request: Request<BoxBody>) -> Result<Response<String>, std::io::Error> {
    let data = 
        request.into_body()
            .data()
            .await
            .transpose()
            .map_err(|_| std::io::Error::new(std::io::ErrorKind::Other, "uh oh".to_string()))?
            .unwrap();
    std::fs::write(
        path,
        data
    )?;
    Ok(Response::new("Hi".into()))
}
