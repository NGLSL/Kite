#[cfg(not(windows))]
compile_error!("Loci plugin supports Windows only");

mod service;

use serde_json::{json, Value};
use std::io::{self, Read, Write};

fn serve(input: &mut impl Read, output: &mut impl Write) -> io::Result<()> {
    let mut service = None;
    while let Some(message) = kite_plugin_sdk::read_message(input) {
        let id = message.get("id").cloned();
        let params = &message["params"];
        let method = message["method"].as_str().unwrap_or("");
        let result: Result<Value, String> = match method {
            "plugin/initialize" if service.is_none() => {
                if params["plugin_api"] != 1 {
                    Err("unsupported plugin_api".into())
                } else if let Some(dir) = params["data_dir"].as_str() {
                    match service::Service::start(dir.into()) {
                        Ok(started) => {
                            service = Some(started);
                            Ok(kite_plugin_sdk::initialize_result())
                        }
                        Err(error) => Err(error.to_string()),
                    }
                } else {
                    Err("data_dir is required".into())
                }
            }
            "plugin/initialize" => Err("already initialized".into()),
            "plugin/query" => service.as_ref().ok_or("initialize first".into()).map(|s| {
                let query = params["query"].as_str().unwrap_or("");
                if params["provider_id"] == "file_search" {
                    s.query_files(query)
                } else {
                    s.query(query)
                }
            }),
            "plugin/execute" => service
                .as_ref()
                .ok_or("initialize first".into())
                .and_then(|s| {
                    s.execute(
                        params["action_id"].as_str().unwrap_or(""),
                        &params["payload"],
                    )
                }),
            "plugin/dispose" => Ok(json!({"ok": true})),
            "$/cancelRequest" => continue,
            _ => Err("method not found".into()),
        };
        if let Some(id) = id {
            let response = match result {
                Ok(value) => kite_plugin_sdk::rpc_result(id, value),
                Err(error) => kite_plugin_sdk::rpc_error(id, -32602, &error),
            };
            kite_plugin_sdk::write_message(output, &response)?;
        }
        if method == "plugin/dispose" {
            break;
        }
    }
    drop(service);
    Ok(())
}

fn main() {
    if let Err(error) = serve(&mut io::stdin().lock(), &mut io::stdout().lock()) {
        eprintln!("Loci protocol: {error}");
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn framed_lifecycle_requires_initialize_and_disposes() {
        let dir = std::env::temp_dir().join(format!("kite-loci-rpc-{}", std::process::id()));
        let mut input = Vec::new();
        for request in [
            json!({"id":1,"method":"plugin/query","params":{"query":""}}),
            json!({"id":2,"method":"plugin/initialize","params":{"plugin_api":1,"data_dir":dir}}),
            json!({"id":3,"method":"plugin/initialize","params":{"plugin_api":1,"data_dir":dir}}),
            json!({"id":4,"method":"plugin/query","params":{"query":""}}),
            json!({"id":5,"method":"plugin/dispose"}),
            json!({"id":6,"method":"plugin/query"}),
        ] {
            kite_plugin_sdk::write_message(&mut input, &request).unwrap();
        }
        let mut output = Vec::new();
        serve(&mut input.as_slice(), &mut output).unwrap();
        let mut frames = output.as_slice();
        assert!(kite_plugin_sdk::read_message(&mut frames).unwrap()["error"].is_object());
        assert_eq!(
            kite_plugin_sdk::read_message(&mut frames).unwrap()["result"]["plugin_api"],
            1
        );
        assert!(kite_plugin_sdk::read_message(&mut frames).unwrap()["error"].is_object());
        assert_eq!(
            kite_plugin_sdk::read_message(&mut frames).unwrap()["result"]["type"],
            "panel"
        );
        assert_eq!(
            kite_plugin_sdk::read_message(&mut frames).unwrap()["result"]["ok"],
            true
        );
        assert!(kite_plugin_sdk::read_message(&mut frames).is_none());
        let _ = std::fs::remove_dir_all(dir);
    }
}
