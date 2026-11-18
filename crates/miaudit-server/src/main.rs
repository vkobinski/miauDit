use clap::{Parser, Subcommand};
use miaudit_server::{AppState, db::Db, router};

#[derive(Parser)]
#[command(name = "miaudit-server", about = "Team server for miauDit audit commits (MongoDB)")]
struct Args {
    #[arg(long, env = "MIAUDIT_MONGO_URI", default_value = "mongodb://127.0.0.1:27017")]
    mongo_uri: String,
    #[arg(long, env = "MIAUDIT_MONGO_DB", default_value = "miaudit")]
    db: String,
    #[command(subcommand)]
    cmd: Cmd,
}

#[derive(Subcommand)]
enum Cmd {
    Serve {
        #[arg(long, default_value = "127.0.0.1")]
        bind: String,
        #[arg(long, default_value_t = 7171)]
        port: u16,
    },
    Token {
        #[command(subcommand)]
        op: TokenOp,
    },
}

#[derive(Subcommand)]
enum TokenOp {
    Create { name: String },
    List,
    Revoke { name: String },
}

fn new_token() -> String {
    format!("mdt_{}{}", uuid::Uuid::new_v4().simple(), uuid::Uuid::new_v4().simple())
}

#[tokio::main]
async fn main() {
    let a = Args::parse();
    let db = match Db::connect(&a.mongo_uri, &a.db).await {
        Ok(d) => d,
        Err(e) => {
            eprintln!("cannot connect to MongoDB: {e:?}");
            std::process::exit(1);
        }
    };
    match a.cmd {
        Cmd::Token { op } => match op {
            TokenOp::Create { name } => {
                let token = new_token();
                match db.create_token(&name, &token).await {
                    Ok(()) => {
                        println!("{token}");
                        eprintln!("token {name:?} created; this is the only time it is shown. Export it as MIAUDIT_TOKEN.");
                    }
                    Err(e) => {
                        eprintln!("{e:?}");
                        std::process::exit(1);
                    }
                }
            }
            TokenOp::List => match db.list_tokens().await {
                Ok(list) => {
                    for t in list {
                        println!("{}\t{}\t{}", t.name, t.created_at, if t.revoked { "revoked" } else { "active" });
                    }
                }
                Err(e) => {
                    eprintln!("{e:?}");
                    std::process::exit(1);
                }
            },
            TokenOp::Revoke { name } => match db.revoke_token(&name).await {
                Ok(true) => println!("revoked {name}"),
                Ok(false) => {
                    eprintln!("no active token named {name}");
                    std::process::exit(1);
                }
                Err(e) => {
                    eprintln!("{e:?}");
                    std::process::exit(1);
                }
            },
        },
        Cmd::Serve { bind, port } => {
            let read_password = std::env::var("MIAUDIT_SERVER_READ_PASSWORD").ok().filter(|p| !p.is_empty());
            let loopback = bind == "127.0.0.1" || bind == "::1" || bind == "localhost";
            if !loopback && read_password.is_none() {
                eprintln!("warning: listening on {bind} with no MIAUDIT_SERVER_READ_PASSWORD; anyone who can reach this port can read every audit");
            }
            let listener = match tokio::net::TcpListener::bind((bind.as_str(), port)).await {
                Ok(l) => l,
                Err(e) => {
                    eprintln!("cannot bind {bind}:{port}: {e}");
                    std::process::exit(1);
                }
            };
            println!("miauDit server on http://{bind}:{port} (database {})", a.db);
            let _ = axum::serve(listener, router(AppState { db, read_password }))
                .with_graceful_shutdown(async {
                    let _ = tokio::signal::ctrl_c().await;
                })
                .await;
        }
    }
}
