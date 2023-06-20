use serde::{Serialize, Deserialize};


// Represents the HTTP request
#[derive(Serialize, Deserialize, Debug, Clone)]
pub struct Request<'a> {
    pub uri: String,
    pub body: &'a [u8]
}


// Represents the HTTP response
#[derive(Serialize, Deserialize, Debug, Clone)]
pub struct Response<'a> {
    pub body: &'a [u8],
    pub status: u16,
}