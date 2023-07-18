use async_trait::async_trait;
use hello_world::{
    greeter_server::{Greeter, GreeterServer},
    HelloReply, HelloRequest,
};
use server::ReceiverStream;
use tonic::{Request, Response, Status};

use crate::server::Server;

pub mod hello_world {
    tonic::include_proto!("helloworld");
}
mod http2;
mod server;

pub struct MyGreeter;

#[async_trait]
impl Greeter for MyGreeter {
    async fn say_hello(
        &self,
        request: Request<HelloRequest>,
    ) -> Result<Response<Self::SayHelloStream>, Status> {
        let name = request.into_inner().name;
        println!("Req from {}", name);
        let (tx, rx) = tokio::sync::mpsc::channel(64);
        for i in 0..5 {
            let reply = hello_world::HelloReply {
                message: format!("{}: Hi {}", i, name),
            };
            tx.send(Ok(reply)).await.unwrap();
        }
        Ok(Response::new(ReceiverStream::new(rx)))
    }
    type SayHelloStream = ReceiverStream<Result<HelloReply, Status>>;
}

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    let greeter = MyGreeter {};
    let greeter_server = GreeterServer::new(greeter);
    let server = Server::new("/services/detector.sock".into(), greeter_server);
    server.serve().await?;

    Ok(())
}
