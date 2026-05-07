use actix_web::{get, App, HttpServer, Responder};

fn very_complex(n: i32) -> i32 {
    let mut out = 0;
    for i in 0..n {
        if i % 2 == 0 { out += i; } else if i % 3 == 0 { out -= i; } else if i % 5 == 0 { out += i * 2; }
        else if i % 7 == 0 { out -= i * 2; } else if i % 11 == 0 { out += 11; } else { out += 1; }
    }
    out
}

#[get("/greet/{name}")]
async fn greet(path: actix_web::web::Path<String>) -> impl Responder {
    format!("Hello, {}! {}", path.into_inner(), very_complex(42))
}

#[actix_web::main]
async fn main() -> std::io::Result<()> {
    HttpServer::new(|| App::new().service(greet))
        .bind(("127.0.0.1", 8080))?
        .run()
        .await
}
