use client::Client;
use hello_world::{greeter_client::GreeterClient, HelloRequest};
use wasm_rs_async_executor::single_threaded as executor;

pub mod hello_world {
    tonic::include_proto!("helloworld");
}

mod client;

fn main() -> Result<(), Box<dyn std::error::Error>> {
    executor::spawn(async move { main_impl().await });
    executor::run(None);
    Ok(())
}

async fn main_impl() -> Result<(), Box<dyn std::error::Error>> {
    let req = tonic::Request::new(HelloRequest { name: "bob".into() });

    let mut client = GreeterClient::new(Client::new("output/client-tx.sock".into()));
    let resp = client.say_hello(req).await;
    println!("Response: {:?}", resp);

    Ok(())
}
