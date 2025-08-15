use anyhow::Result;
use colored::Colorize;
use reqwest::{header::HeaderMap, Response, StatusCode, Version};
use serde_json::Value;
use std::{
    collections::HashMap,
    fmt::Write,
};

pub struct ResponseFormatter {
    pub colorize: bool,
    pub format_json: bool,
    pub format_xml: bool,
}

impl Default for ResponseFormatter {
    fn default() -> Self {
        Self {
            colorize: true,
            format_json: true,
            format_xml: false,
        }
    }
}

impl ResponseFormatter {
    pub fn new(colorize: bool, format_json: bool, format_xml: bool) -> Self {
        Self {
            colorize,
            format_json,
            format_xml,
        }
    }

    pub fn format_status_line(&self, version: Version, status: StatusCode) -> String {
        let version_str = format!("{:?}", version);
        let status_str = format!("{} {}", status.as_u16(), status.canonical_reason().unwrap_or("Unknown"));

        if self.colorize {
            let colored_status = if status.is_success() {
                status_str.green()
            } else if status.is_client_error() {
                status_str.yellow()
            } else if status.is_server_error() {
                status_str.red()
            } else {
                status_str.normal()
            };
            format!("{} {}", version_str.blue(), colored_status)
        } else {
            format!("{} {}", version_str, status_str)
        }
    }

    pub fn format_headers(&self, headers: &HeaderMap) -> String {
        let mut output = String::new();
        for (name, value) in headers {
            let header_line = format!("{}: {}", name, value.to_str().unwrap_or("<invalid>"));
            if self.colorize {
                writeln!(&mut output, "{}", header_line.cyan()).unwrap();
            } else {
                writeln!(&mut output, "{}", header_line).unwrap();
            }
        }
        output
    }

    pub fn format_body(&self, content: &str, content_type: Option<&str>) -> String {
        if let Some(ct) = content_type {
            if self.format_json && ct.contains("json") {
                return self.format_json_content(content);
            }
            if self.format_xml && (ct.contains("xml") || ct.contains("html")) {
                return self.format_xml_content(content);
            }
        }

        // Try to detect JSON even without proper content-type
        if self.format_json && (content.trim_start().starts_with('{') || content.trim_start().starts_with('[')) {
            let formatted = self.format_json_content(content);
            if !formatted.is_empty() && formatted != content {
                return formatted;
            }
        }

        content.to_string()
    }

    fn format_json_content(&self, content: &str) -> String {
        match serde_json::from_str::<Value>(content) {
            Ok(value) => {
                match serde_json::to_string_pretty(&value) {
                    Ok(formatted) => {
                        if self.colorize {
                            self.colorize_json(&formatted)
                        } else {
                            formatted
                        }
                    }
                    Err(_) => content.to_string(),
                }
            }
            Err(_) => content.to_string(),
        }
    }

    fn format_xml_content(&self, content: &str) -> String {
        // Basic XML formatting - in a real implementation you might use a proper XML parser
        content.to_string()
    }

    fn colorize_json(&self, json: &str) -> String {
        // Simple JSON colorization
        let mut result = String::new();
        let mut in_string = false;
        let mut escape_next = false;

        for ch in json.chars() {
            if escape_next {
                result.push(ch);
                escape_next = false;
                continue;
            }

            match ch {
                '"' if !escape_next => {
                    in_string = !in_string;
                    if in_string {
                        result.push_str(&format!("{}", ch.to_string().green()));
                    } else {
                        result.push_str(&format!("{}", ch.to_string().green()));
                    }
                }
                '\\' if in_string => {
                    escape_next = true;
                    result.push(ch);
                }
                _ if in_string => {
                    result.push_str(&format!("{}", ch.to_string().green()));
                }
                ':' => result.push_str(&format!("{}", ch.to_string().blue())),
                ',' => result.push_str(&format!("{}", ch.to_string().white())),
                '{' | '}' | '[' | ']' => result.push_str(&format!("{}", ch.to_string().yellow())),
                _ => result.push(ch),
            }
        }

        result
    }

    pub async fn format_response(&self, response: Response) -> Result<String> {
        let mut output = String::new();

        // Status line
        let status = response.status();
        let version = response.version();
        writeln!(&mut output, "{}", self.format_status_line(version, status))?;

        // Headers
        let headers = response.headers().clone();
        output.push_str(&self.format_headers(&headers));
        output.push('\n');

        // Body
        let content_type = headers.get("content-type")
            .and_then(|ct| ct.to_str().ok());

        let body = response.text().await?;
        output.push_str(&self.format_body(&body, content_type));

        Ok(output)
    }
}

pub struct ResponseAnalyzer;

impl ResponseAnalyzer {
    pub fn analyze_headers(headers: &HeaderMap) -> HashMap<String, String> {
        let mut analysis = HashMap::new();

        // Security headers analysis
        let security_headers = vec![
            "strict-transport-security",
            "content-security-policy",
            "x-frame-options",
            "x-content-type-options",
            "x-xss-protection",
        ];

        for header in security_headers {
            if headers.contains_key(header) {
                analysis.insert(
                    format!("security.{}", header),
                    "present".to_string()
                );
            } else {
                analysis.insert(
                    format!("security.{}", header),
                    "missing".to_string()
                );
            }
        }

        // Server information
        if let Some(server) = headers.get("server") {
            analysis.insert(
                "server.type".to_string(),
                server.to_str().unwrap_or("unknown").to_string()
            );
        }

        // Caching information
        if let Some(cache_control) = headers.get("cache-control") {
            analysis.insert(
                "cache.control".to_string(),
                cache_control.to_str().unwrap_or("unknown").to_string()
            );
        }

        analysis
    }

    pub fn get_response_summary(
        status: StatusCode,
        headers: &HeaderMap,
        body_size: usize,
        response_time: u64,
    ) -> String {
        format!(
            "Status: {} | Size: {} bytes | Time: {}ms | Server: {}",
            status,
            body_size,
            response_time,
            headers.get("server")
                .and_then(|s| s.to_str().ok())
                .unwrap_or("unknown")
        )
    }
}