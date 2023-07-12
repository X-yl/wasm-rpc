use std::{time::{Instant, SystemTime}, hint::black_box};

use client::Client;
use hello_world::{greeter_client::GreeterClient, HelloRequest};
use wasm_rs_async_executor::single_threaded as executor;

pub mod hello_world {
    tonic::include_proto!("helloworld");
}

mod client;
mod transfer;
mod http2;

fn main() -> Result<(), Box<dyn std::error::Error>> {
    executor::spawn(async move { if let Err(e) = main_impl().await {
        println!("Client err: {:?}", e);
    }});
    executor::run(None);
    Ok(())
}

async fn main_impl() -> Result<(), Box<dyn std::error::Error>> {
    let mut avg_time_micros = 0;
    for i in 0..1000{
        let start = SystemTime::now();
        let mut client = GreeterClient::new(Client::new("/services/detector.sock".into()));
        let req = tonic::Request::new(HelloRequest { name: format!("bob{}", i) });
        let resp = client.say_hello(req).await?;
        
        let elapsed = SystemTime::now().duration_since(start)?.as_micros();
        if avg_time_micros == 0 {
            avg_time_micros = elapsed;
        } else {
            avg_time_micros = (avg_time_micros + elapsed)/2
        }
    }
    
    println!("Average time {}us", avg_time_micros);

    Ok(())
}
