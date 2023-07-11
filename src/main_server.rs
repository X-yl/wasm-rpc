use async_trait::async_trait;
use hello_world::{
    greeter_server::{Greeter, GreeterServer},
    HelloReply, HelloRequest,
};
use tonic::{
    Request, Response, Status,
};
use wasm_rs_async_executor::single_threaded as executor;

use crate::server::Server;

pub mod hello_world {
    tonic::include_proto!("helloworld");
}

mod transfer;
mod server;

pub struct MyGreeter;

#[async_trait]
impl Greeter for MyGreeter {
    async fn say_hello(
        &self,
        request: Request<HelloRequest>,
    ) -> Result<Response<HelloReply>, Status> {
        let name = request.into_inner().name;
        println!("Got a request from {:?}", name);

        let reply = hello_world::HelloReply {
            message: format!("Hello {}", name),
        };
        Ok(Response::new(reply))
    }
}

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let handle = executor::spawn(main_impl());
    executor::run(Some(handle.task()));
    Ok(())
}

async fn main_impl() -> Result<(), Box<dyn std::error::Error>> {
    let greeter = MyGreeter {};
    let greeter_server = GreeterServer::new(greeter);
    let server = Server::new("/services/detector.sock".into(), greeter_server);
    server.serve().await?;

    Ok(())
}
