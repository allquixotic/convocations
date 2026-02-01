//! OAuth flow implementation

use std::sync::{Arc, Mutex};
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::TcpListener;

/// OAuth flow state
pub struct OAuthFlow {
    result: Arc<Mutex<Option<Result<String, String>>>>,
}

impl OAuthFlow {
    /// Start OAuth flow
    pub fn start() -> Result<Self, String> {
        // Generate PKCE pair
        let (code_verifier, code_challenge) = rconv_core::openrouter::generate_pkce_pair();

        // Build OAuth URL
        let callback_url = "http://localhost:8787/callback";
        let oauth_url = rconv_core::openrouter::build_oauth_url(
            &code_challenge,
            callback_url,
            None,
            Some("Convocations"),
        );

        // Open browser
        if let Err(e) = open::that(&oauth_url) {
            return Err(format!("Failed to open browser: {}", e));
        }

        let result = Arc::new(Mutex::new(None));
        let result_clone = result.clone();
        let verifier_clone = code_verifier.clone();

        // Start callback server in background
        tokio::spawn(async move {
            if let Err(e) = run_callback_server(verifier_clone, result_clone).await {
                eprintln!("OAuth callback server error: {}", e);
            }
        });

        Ok(Self {
            result,
        })
    }

    /// Check if OAuth flow is complete
    pub fn poll(&self) -> Option<Result<String, String>> {
        let mut result = self.result.lock().unwrap();
        result.take()
    }
}

/// Run OAuth callback server
async fn run_callback_server(
    code_verifier: String,
    result: Arc<Mutex<Option<Result<String, String>>>>,
) -> Result<(), String> {
    let listener = TcpListener::bind("127.0.0.1:8787")
        .await
        .map_err(|e| format!("Failed to bind to port 8787: {}", e))?;

    // Accept one connection
    let (mut stream, _) = listener
        .accept()
        .await
        .map_err(|e| format!("Failed to accept connection: {}", e))?;

    // Read request
    let mut buffer = vec![0u8; 4096];
    let n = stream
        .read(&mut buffer)
        .await
        .map_err(|e| format!("Failed to read request: {}", e))?;

    let request = String::from_utf8_lossy(&buffer[..n]);

    // Parse code from request
    let code = if let Some(query_start) = request.find("/callback?") {
        let query = &request[query_start + 10..];
        if let Some(code_start) = query.find("code=") {
            let code_part = &query[code_start + 5..];
            if let Some(code_end) = code_part.find(|c: char| c.is_whitespace() || c == '&') {
                Some(code_part[..code_end].to_string())
            } else {
                None
            }
        } else {
            None
        }
    } else {
        None
    };

    // Send response
    let response = if code.is_some() {
        "HTTP/1.1 200 OK\r\n\
         Content-Type: text/html\r\n\
         \r\n\
         <html><body>\
         <h1>Authorization Successful!</h1>\
         <p>You can close this window and return to Convocations.</p>\
         </body></html>"
    } else {
        "HTTP/1.1 400 Bad Request\r\n\
         Content-Type: text/html\r\n\
         \r\n\
         <html><body>\
         <h1>Authorization Failed</h1>\
         <p>No authorization code received.</p>\
         </body></html>"
    };

    stream
        .write_all(response.as_bytes())
        .await
        .map_err(|e| format!("Failed to send response: {}", e))?;
    stream
        .flush()
        .await
        .map_err(|e| format!("Failed to flush stream: {}", e))?;

    // Exchange code for API key if we got one
    if let Some(code_value) = code {
        match rconv_core::openrouter::exchange_code_for_api_key(&code_value, &code_verifier).await {
            Ok(api_key) => {
                *result.lock().unwrap() = Some(Ok(api_key));
            }
            Err(e) => {
                *result.lock().unwrap() = Some(Err(format!("Failed to exchange code: {}", e)));
            }
        }
    } else {
        *result.lock().unwrap() = Some(Err("No authorization code received".to_string()));
    }

    Ok(())
}
