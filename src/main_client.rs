use std::{
    hint::black_box,
    time::{Instant, SystemTime},
};

use client::Client;
use hello_world::{greeter_client::GreeterClient, HelloRequest};
use wasm_rs_async_executor::single_threaded as executor;

pub mod hello_world {
    tonic::include_proto!("helloworld");
}

mod client;
mod http2;

fn main() -> Result<(), Box<dyn std::error::Error>> {
    executor::spawn(async move {
        if let Err(e) = main_impl().await {
            println!("Client err: {:?}", e);
        }
    });
    executor::run(None);
    Ok(())
}

async fn main_impl() -> Result<(), Box<dyn std::error::Error>> {
    std::fs::write("/services/detector.special", "pls start :)");
    let mut avg_time_micros = 0;
    for i in 0..2 {
        let start = SystemTime::now();
        let mut client = GreeterClient::new(Client::new("/services/detector.sock".into()));
        let req = tonic::Request::new(HelloRequest {
            name: format!("bob{}", i),
        });
        let mut resp = client.say_hello(req).await?;
        while let Ok(Some(msg)) = resp.get_mut().message().await {
            println!("{:?}", msg);
        }

        let elapsed = SystemTime::now().duration_since(start)?.as_micros();
        if avg_time_micros == 0 {
            avg_time_micros = elapsed;
        } else {
            avg_time_micros = (avg_time_micros + elapsed) / 2
        }
    }

    println!("Average time {}us", avg_time_micros);

    Ok(())
}
