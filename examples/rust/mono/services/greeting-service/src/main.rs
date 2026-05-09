use actix_web::{get, web, App, HttpServer, Responder};

#[get("/greet/{name}")]
async fn greet(name: web::Path<String>) -> impl Responder {
    let n = mathlib::square(3);
    format!("{} Number={n}", greetinglib::salute(&name))
}

#[actix_web::main]
async fn main() -> std::io::Result<()> {
    HttpServer::new(|| App::new().service(greet)).bind(("127.0.0.1", 8081))?.run().await
}
