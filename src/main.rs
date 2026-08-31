// src/main.rs
use aegis_ai::domain::{LlmProvider, LlmRequest};
use aegis_ai::provider::OllamaProvider;

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    let provider = OllamaProvider::new("http://localhost:11434", "llama3");

    let request = LlmRequest {
        system_prompt: "You respond ONLY in valid JSON with key 'status'".to_string(),
        user_prompt: "Give me an operational ping".to_string(),
    };

    println!("Sending request to Ollama...");
    match provider.generate(&request).await {
        Ok(res) => {
            println!("Response: {}", res.output);
            println!("Latency: {} ms", res.latency_ms);
        }
        Err(e) => eprintln!("Error: {}", e),
    }

    Ok(())
}
