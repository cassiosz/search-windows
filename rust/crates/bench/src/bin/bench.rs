//! `bench state | open URL | type TEXT | key KEYS | eval JS | wait loaded [ms] | quit`
//! Talks to the running app's bench pipe, prints the response, exits 1 on an error.

use searchwin_bench::{Request, call};

fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let rest = args.get(1..).unwrap_or_default().join(" ");
    let request = match args.first().map(String::as_str) {
        Some("state") => Request::State,
        Some("open") => Request::Open { url: rest },
        Some("type") => Request::Type { text: rest },
        Some("key") => Request::Key { keys: rest },
        Some("eval") => Request::Eval { js: rest },
        Some("wait") => Request::Wait {
            until: args.get(1).cloned().unwrap_or_else(|| "loaded".into()),
            timeout_ms: args.get(2).and_then(|t| t.parse().ok()).unwrap_or(5000),
        },
        Some("quit") => Request::Quit,
        _ => {
            eprintln!("usage: bench state | open URL | type TEXT | key KEYS | eval JS | wait loaded [ms] | quit");
            std::process::exit(2);
        }
    };
    match call(&request) {
        Ok(response) => {
            println!("{}", serde_json::to_string(&response).unwrap_or_default());
            if !response.ok {
                std::process::exit(1);
            }
        }
        Err(e) => {
            eprintln!("bench: no app to talk to ({e}). Start it with SEARCHWIN_BENCH=1.");
            std::process::exit(1);
        }
    }
}
