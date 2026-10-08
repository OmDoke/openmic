#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

use tauri::{AppHandle, Emitter, Manager, State};
use tauri_plugin_global_shortcut::{Code, Modifiers, Shortcut, ShortcutState};
use serde_json::json;
use std::sync::Mutex;
use xcap::Monitor;
use base64::{Engine as _, engine::general_purpose::STANDARD};
use std::io::Cursor;
use reqwest::multipart;
use std::fs;

#[derive(Default)]
struct SpeechState {
    is_recording: Mutex<bool>,
    segment_buffer: Mutex<Vec<u8>>,
    vad_speaking: Mutex<bool>,
    vad_speech_ms: Mutex<f64>,
    vad_silence_ms: Mutex<f64>,
    vad_noise_floor: Mutex<f64>,
    vad_noise_init: Mutex<bool>,
    vad_pre_roll: Mutex<Vec<Vec<u8>>>,
    vad_pre_roll_ms: Mutex<f64>,
    loopback_stream: Mutex<Option<cpal::Stream>>,
}

struct AppState {
    history: Mutex<Vec<serde_json::Value>>,
    active_skill: Mutex<String>,
    is_interactive: Mutex<bool>,
    answer_held: Mutex<bool>,
    transcription_buffer: Mutex<String>,
    transcription_count: Mutex<u32>,
    speech: SpeechState,
}

fn switch_to_chat_window(app: AppHandle) {
    if let Some(window) = app.get_webview_window("llmResponse") {
        let _ = window.show();
        let _ = window.set_focus();
    } else {
        let _win = tauri::WebviewWindowBuilder::new(
            &app,
            "llmResponse",
            tauri::WebviewUrl::App("chat.html".into())
        )
        .title("Chat")
        .inner_size(380.0, 750.0)
        .transparent(true)
        .decorations(false)
        .always_on_top(true)
        .resizable(false)
        .skip_taskbar(true)
        .visible(false)
        .content_protected(true)
        .build()
        .unwrap();
    }
}

fn switch_to_llm_response_window(app: AppHandle) {
    if let Some(window) = app.get_webview_window("llmResponseWindow") {
        let _ = window.show();
        let _ = window.set_focus();
    } else {
        if let Ok(win) = tauri::WebviewWindowBuilder::new(
            &app,
            "llmResponseWindow",
            tauri::WebviewUrl::App("llm-response.html".into())
        )
        .title("AI Response")
        .inner_size(950.0, 750.0)
        .transparent(true)
        .decorations(false)
        .always_on_top(true)
        .resizable(true)
        .skip_taskbar(true)
        .visible(true)
        .content_protected(true)
        .build() {
            let _ = win.set_focus();
        }
    }
}

fn switch_to_live_answer_window(app: AppHandle) {
    if let Some(window) = app.get_webview_window("liveAnswer") {
        let _ = window.show();
    } else {
        let _win = tauri::WebviewWindowBuilder::new(
            &app,
            "liveAnswer",
            tauri::WebviewUrl::App("live-answer.html".into())
        )
        .title("Live Answer")
        .inner_size(650.0, 300.0)
        .transparent(true)
        .decorations(false)
        .always_on_top(true)
        .resizable(false)
        .skip_taskbar(true)
        .visible(false)
        .content_protected(true)
        .build()
        .unwrap();
    }
}

#[tauri::command]
fn notify_live_answer_window_ready(app: tauri::AppHandle) {
    if let Some(win) = app.get_webview_window("liveAnswer") {
        let _ = win.show();
    }
}

#[tauri::command]
fn hide_live_answer_window(app: AppHandle) {
    if let Some(window) = app.get_webview_window("liveAnswer") {
        let _ = window.hide();
    }
}

fn get_system_prompt(skill: &str) -> String {
    let path = format!("../src/prompts/{}.md", skill);
    fs::read_to_string(&path).unwrap_or_else(|_| "You are a helpful coding assistant. Answer concisely.".to_string())
}

#[tauri::command]
fn show_all_windows(app: AppHandle) {
    for (_, window) in app.webview_windows() {
        let _ = window.show();
    }
}

#[tauri::command]
fn hide_all_windows(app: AppHandle) {
    for (_, window) in app.webview_windows() {
        let _ = window.hide();
    }
}

#[tauri::command]
fn enable_window_interaction(app: AppHandle, state: State<'_, AppState>) {
    *state.is_interactive.lock().unwrap() = true;
    for (_, window) in app.webview_windows() {
        let _ = window.set_ignore_cursor_events(false);
    }
}

#[tauri::command]
fn disable_window_interaction(app: AppHandle, state: State<'_, AppState>) {
    *state.is_interactive.lock().unwrap() = false;
    for (_, window) in app.webview_windows() {
        let _ = window.set_ignore_cursor_events(true);
    }
}

#[tauri::command]
async fn switch_to_chat(app: tauri::AppHandle) {
    switch_to_chat_window(app.clone());
    switch_to_live_answer_window(app);
}

#[tauri::command] fn get_speech_availability() -> bool { true }
#[tauri::command]
fn get_settings(app: tauri::AppHandle) -> serde_json::Value {
    use tauri::Manager;
    if let Ok(path) = app.path().app_data_dir() {
        let settings_path = path.join("settings.json");
        if let Ok(data) = std::fs::read_to_string(settings_path) {
            if let Ok(json) = serde_json::from_str(&data) {
                return json;
            }
        }
    }
    serde_json::json!({})
}

fn get_env_or_setting(app: &tauri::AppHandle, setting_key: &str, env_key: &str, default: &str) -> String {
    let settings = get_settings(app.clone());
    if let Some(val) = settings.get(setting_key).and_then(|v| v.as_str()) {
        if !val.trim().is_empty() {
            return val.trim().to_string();
        }
    }
    if let Ok(val) = std::env::var(env_key) {
        if !val.trim().is_empty() {
            return val.trim().to_string();
        }
    }
    let fallback_paths = [
        "c:\\Users\\Admin\\Desktop\\openmic\\.env",
        ".env",
    ];
    for p in &fallback_paths {
        if let Ok(content) = std::fs::read_to_string(p) {
            for line in content.lines() {
                let line = line.trim();
                if line.starts_with('#') || !line.contains('=') { continue; }
                let mut parts = line.splitn(2, '=');
                if let (Some(k), Some(v)) = (parts.next(), parts.next()) {
                    if k.trim() == env_key && !v.trim().is_empty() {
                        return v.trim().to_string();
                    }
                }
            }
        }
    }
    default.to_string()
}

#[tauri::command]
fn take_screenshot(app: AppHandle, state: State<'_, AppState>) {
    let skill = state.active_skill.lock().unwrap().clone();
    let prompt = get_system_prompt("dsa"); // Hardcoded to dsa as requested
    
    tauri::async_runtime::spawn(async move {
        let emit_error = {
            let app = app.clone();
            move |err_msg: &str| {
                let msg = err_msg.to_string();
                let app_c = app.clone();
                tauri::async_runtime::spawn(async move {
                    tokio::time::sleep(std::time::Duration::from_millis(800)).await;
                    let _ = app_c.emit("llm-error", serde_json::json!({ "error": msg.clone() }));
                    if let Some(win) = app_c.get_webview_window("llmResponseWindow") {
                        let _ = win.show();
                        let escaped_err = serde_json::to_string(&msg).unwrap_or_else(|_| "\"API Error\"".to_string());
                        let err_js = format!(
                            r#"setTimeout(() => {{
                                if (typeof hideLoadingState === 'function') hideLoadingState();
                                const md = document.getElementById('full-markdown');
                                if (md) md.innerHTML = '<div style="color: #ef4444; padding: 20px;"><h2 style="color: #ef4444; margin-top: 0;">Error</h2><p>' + {} + '</p></div>';
                                const split = document.getElementById('split-layout');
                                if (split) split.classList.add('hidden');
                                const full = document.getElementById('full-content');
                                if (full) full.classList.remove('hidden');
                                const resp = document.getElementById('response-content');
                                if (resp) resp.classList.remove('hidden');
                            }}, 100);"#,
                            escaped_err
                        );
                        let _ = win.eval(&err_js);
                    }
                });
            }
        };

        let b64 = {
            let monitors = match Monitor::all() {
                Ok(m) => m,
                Err(e) => { emit_error(&format!("Monitor::all() failed: {}", e)); return; }
            };
            if monitors.is_empty() {
                emit_error("No monitors found!");
                return;
            }
            let primary = match monitors.first() {
                Some(m) => m,
                None => { emit_error("monitors.first() returned None!"); return; }
            };
            let image = match primary.capture_image() {
                Ok(i) => i,
                Err(e) => { emit_error(&format!("capture_image() failed: {}", e)); return; }
            };

            let mut buf = std::io::Cursor::new(Vec::new());
            if let Err(e) = image.write_to(&mut buf, image::ImageFormat::Png) {
                emit_error(&format!("write_to failed: {}", e));
                return;
            }
            let b64_str = STANDARD.encode(buf.into_inner());
            println!("Captured image. Base64 length: {} bytes", b64_str.len());
            b64_str
        }; 

        // Open llm-response window AFTER capturing the screenshot so it doesn't capture itself
        switch_to_llm_response_window(app.clone());

        let message_id = format!("img-{}", std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).unwrap().as_millis());
        let _ = app.emit("show-loading", serde_json::json!({
            "messageId": message_id,
            "skill": skill
        }));

        // Live-reload .env so users don't have to restart the app
        let _ = dotenvy::dotenv_override().ok();
        let api_key = get_env_or_setting(&app, "geminiKey", "GEMINI_API_KEY", "");
        let model = get_env_or_setting(&app, "geminiModel", "GEMINI_MODEL", "gemini-3.5-flash-lite");
        println!("API Key empty? {}", api_key.is_empty());
        println!("Model: {}", model);
        if api_key.is_empty() { 
            emit_error("GEMINI_API_KEY is not set! Click Settings (gear icon) on the top bar and enter your Gemini API Key.");
            return; 
        }

        let client = reqwest::Client::builder()
            .danger_accept_invalid_certs(true)
            .user_agent("Mozilla/5.0 (Windows NT 10.0; Win64; x64) AppleWebKit/537.36 (KHTML, like Gecko) Chrome/122.0.0.0 Safari/537.36")
            .timeout(std::time::Duration::from_secs(90))
            .build()
            .unwrap_or_else(|_| reqwest::Client::new());
            
        // Track when we started so we can ensure the window has time to load
        let start_time = std::time::Instant::now();
        
        let body = serde_json::json!({
            "system_instruction": {
                "parts": [{"text": prompt}]
            },
            "contents": [{
                "parts": [
                    { "text": "Analyze this screenshot based on your system instructions." },
                    {
                        "inline_data": {
                            "mime_type": "image/png",
                            "data": b64
                        }
                    }
                ]
            }],
            "generationConfig": {
                "maxOutputTokens": 8192
            }
        });

        let mut models_to_try = vec![
            model.clone(),
            "gemini-3.5-flash-lite".to_string(),
            "gemini-3.1-flash-lite".to_string(),
            "gemini-3.5-flash".to_string(),
            "gemini-flash-latest".to_string(),
            "gemini-pro-latest".to_string()
        ];
        models_to_try.dedup();
        
        for try_model in models_to_try {
            let url = format!("https://generativelanguage.googleapis.com/v1beta/models/{}:streamGenerateContent?alt=sse&key={}", try_model, api_key);
            println!("Uploading screenshot to model: {}", try_model);
            let res = client.post(&url).json(&body).send().await;
            
            match res {
                Ok(mut r) => {
                    let status = r.status().as_u16();
                    println!("Headers received! Status: {}", status);
                    if status == 503 || status == 429 || status == 404 || status == 400 {
                        println!("API returned {} for {}. Trying fallback model...", status, try_model);
                        tokio::time::sleep(std::time::Duration::from_millis(500)).await;
                        continue;
                    }

                    // Ensure at least 1500ms has passed since window creation so it can attach IPC listeners
                    let elapsed = start_time.elapsed();
                    if elapsed < std::time::Duration::from_millis(1500) {
                        tokio::time::sleep(std::time::Duration::from_millis(1500) - elapsed).await;
                    }

                    let mut is_retryable_error = false;
                    let mut response_text = String::new();
                    let mut buffer = String::new();
                    let mut first_chunk_processed = false;
                    let mut start_emitted = false;
                    let mut start_emitted_live = false;

                    while let Ok(Some(chunk)) = r.chunk().await {
                        if let Ok(text) = std::str::from_utf8(&chunk) {
                            if !first_chunk_processed {
                                first_chunk_processed = true;
                                let trimmed = text.trim_start();
                                if trimmed.starts_with('{') || trimmed.starts_with("[\n  {\n    \"error\"") {
                                    if text.contains("\"code\": 503") || text.contains("\"code\":503") || text.contains("\"code\": 429") || text.contains("\"code\": 404") || text.contains("\"code\": 400") || text.contains("\"status\": \"UNAVAILABLE\"") || text.contains("\"status\": \"NOT_FOUND\"") {
                                        println!("API returned JSON error for {}. Trying fallback model...", try_model);
                                        is_retryable_error = true;
                                        break;
                                    } else {
                                        emit_error(&format!("API Error: {}", text));
                                        return;
                                    }
                                }
                            }

                            buffer.push_str(text);
                            while let Some(pos) = buffer.find('\n') {
                                let line = buffer[..pos].to_string();
                                buffer.drain(..=pos);
                                
                                let line = line.trim();
                                if line.starts_with("data: ") {
                                    let json_str = &line[6..];
                                    if json_str == "[DONE]" { continue; }
                                    
                                    if let Ok(json_res) = serde_json::from_str::<serde_json::Value>(json_str) {
                                        if let Some(candidates) = json_res.get("candidates") {
                                            if let Some(first) = candidates.get(0) {
                                                if let Some(content) = first.get("content") {
                                                    if let Some(parts) = content.get("parts") {
                                                        if let Some(part) = parts.get(0) {
                                                            if let Some(text_val) = part.get("text") {
                                                                if let Some(new_text) = text_val.as_str() {
                                                                    println!("Extracted text: {}", new_text);
                                                                    response_text.push_str(new_text);
                                                                    
                                                                    if let Some(win) = app.get_webview_window("llmResponseWindow") {
                                                                        if !start_emitted {
                                                                            let _ = win.emit("transcription-llm-response-start", serde_json::json!({
                                                                                "messageId": message_id.clone()
                                                                            }));
                                                                            start_emitted = true;
                                                                        }
                                                                        let _ = win.emit("transcription-llm-response-chunk", serde_json::json!({
                                                                            "messageId": message_id.clone(),
                                                                            "delta": new_text,
                                                                            "textSoFar": response_text.clone()
                                                                        }));
                                                                    }
                                                                    if let Some(win) = app.get_webview_window("liveAnswer") {
                                                                        if !start_emitted_live {
                                                                            let _ = win.emit("transcription-llm-response-start", serde_json::json!({
                                                                                "messageId": message_id.clone()
                                                                            }));
                                                                            start_emitted_live = true;
                                                                        }
                                                                        let _ = win.emit("transcription-llm-response-chunk", serde_json::json!({
                                                                            "messageId": message_id.clone(),
                                                                            "delta": new_text,
                                                                            "textSoFar": response_text.clone()
                                                                        }));
                                                                    }
                                                                }
                                                            }
                                                        }
                                                    }
                                                }
                                            }
                                        }
                                    }
                                }
                            }
                        }
                    }
                    
                    if !buffer.is_empty() {
                        let line = buffer.trim();
                        if line.starts_with("data: ") {
                            let json_str = &line[6..];
                            if json_str != "[DONE]" {
                                if let Ok(json_res) = serde_json::from_str::<serde_json::Value>(json_str) {
                                    if let Some(candidates) = json_res.get("candidates") {
                                        if let Some(first) = candidates.get(0) {
                                            if let Some(content) = first.get("content") {
                                                if let Some(parts) = content.get("parts") {
                                                    if let Some(part) = parts.get(0) {
                                                        if let Some(text_val) = part.get("text") {
                                                            if let Some(new_text) = text_val.as_str() {
                                                                println!("Extracted text (final): {}", new_text);
                                                                response_text.push_str(new_text);
                                                            }
                                                        }
                                                    }
                                                }
                                            }
                                        }
                                    }
                                }
                            }
                        }
                    }

                    if is_retryable_error {
                        tokio::time::sleep(std::time::Duration::from_millis(500)).await;
                        continue;
                    }

                    if response_text.is_empty() {
                        emit_error("Failed to parse Gemini API response JSON (stream was empty or invalid).");
                    } else {
                        if let Some(win) = app.get_webview_window("llmResponseWindow") {
                            // Ensure the very last chunk is emitted specifically to this window
                            let _ = win.emit("display-llm-response", serde_json::json!({
                                "messageId": message_id.clone(),
                                "content": response_text.clone()
                            }));
                            
                            // Foolproof fallback: directly inject the response into the UI via JS eval
                            let escaped_json = serde_json::to_string(&response_text).unwrap_or_else(|_| "\"Error escaping text\"".to_string());
                            let js_code = format!(
                                "if (typeof displayResponse === 'function') {{ hideLoadingState(); displayResponse({{ content: {} }}); setupScrolling(); }}", 
                                escaped_json
                            );
                            let _ = win.eval(&js_code);
                            
                            println!("Stream completed. Emitted final text to UI (and injected via eval).");
                        } else {
                            println!("ERROR: llm-response window not found! Cannot update UI.");
                        }
                        if let Some(win) = app.get_webview_window("liveAnswer") {
                            let _ = win.show();
                            let _ = win.emit("display-llm-response", serde_json::json!({
                                "messageId": message_id.clone(),
                                "content": response_text.clone()
                            }));
                            // Foolproof fallback for liveAnswer
                            let escaped_live = serde_json::to_string(&response_text).unwrap_or_else(|_| "\"\"".to_string());
                            let live_js = format!(
                                r#"setTimeout(() => {{
                                    const el = document.getElementById('answerContent');
                                    if (el) {{
                                        const ph = document.getElementById('placeholder');
                                        if (ph) ph.style.display = 'none';
                                        if (window.marked) {{
                                            el.innerHTML = marked.parse({});
                                        }} else {{
                                            el.textContent = {};
                                        }}
                                        el.scrollTop = 0;
                                    }}
                                }}, 200);"#,
                                escaped_live, escaped_live
                            );
                            let _ = win.eval(&live_js);
                        }
                    }
                    return; // Successfully completed streaming for this model
                },
                Err(e) => {
                    println!("Failed to send request: {:?}", e);
                    continue; // Try next fallback model
                }
            }
        }
        
        emit_error("All fallback models failed (Connection Error / Timeout / 503).");
    });
}
#[tauri::command]
async fn start_speech_recognition(app: AppHandle, state: State<'_, AppState>) -> Result<(), String> {
    *state.speech.is_recording.lock().unwrap() = true;
    *state.speech.vad_speaking.lock().unwrap() = false;
    *state.speech.vad_speech_ms.lock().unwrap() = 0.0;
    *state.speech.vad_silence_ms.lock().unwrap() = 0.0;
    *state.speech.vad_noise_floor.lock().unwrap() = 0.0;
    *state.speech.vad_noise_init.lock().unwrap() = false;
    state.speech.vad_pre_roll.lock().unwrap().clear();
    *state.speech.vad_pre_roll_ms.lock().unwrap() = 0.0;
    state.speech.segment_buffer.lock().unwrap().clear();
    *state.speech.loopback_stream.lock().unwrap() = None;

    let settings = get_settings(app.clone());
    let source = settings.get("captureSource")
        .and_then(|s| s.as_str())
        .map(|s| s.to_string())
        .unwrap_or_else(|| std::env::var("CAPTURE_SOURCE").unwrap_or_else(|_| "system_audio".to_string()));

    if source == "system_audio" {
            #[cfg(target_os = "windows")]
            {
                if let Some(stream) = loopback::start_system_audio_capture(app.clone()) {
                    *state.speech.loopback_stream.lock().unwrap() = Some(stream);
                } else {
                    let emit_error = |err_msg: &str| {
                        let _ = app.emit("llm-error", serde_json::json!({ "error": err_msg }));
                    };
                    emit_error("Failed to start system audio capture. Your audio driver might not be supported, or no audio is playing.");
                }
            }
    } else if source == "microphone" {
            #[cfg(target_os = "windows")]
            {
                if let Some(stream) = loopback::start_microphone_capture(app.clone()) {
                    *state.speech.loopback_stream.lock().unwrap() = Some(stream);
                } else {
                    let emit_error = |err_msg: &str| {
                        let _ = app.emit("llm-error", serde_json::json!({ "error": err_msg }));
                    };
                    emit_error("Failed to start microphone capture. Check your default microphone settings.");
                }
            }
    }
    
    let _ = app.emit("speech-status", json!({ "status": "Waiting for microphone audio...", "available": true }));
    let _ = app.emit("recording-started", ());
    Ok(())
}

#[tauri::command]
async fn stop_speech_recognition(app: AppHandle, state: State<'_, AppState>) -> Result<(), String> {
    *state.speech.is_recording.lock().unwrap() = false;
    *state.speech.loopback_stream.lock().unwrap() = None;
    
    let buf = state.speech.segment_buffer.lock().unwrap().clone();
    state.speech.segment_buffer.lock().unwrap().clear();
    
    if buf.len() > 16000 {
        process_speech_segment(app.clone(), buf);
    }
    
    let _ = app.emit("speech-status", json!({ "status": "Recording stopped", "available": true }));
    let _ = app.emit("recording-stopped", ());
    Ok(())
}

#[tauri::command] fn switch_to_skills() {}
#[tauri::command]
fn resize_window(app: AppHandle, width: f64, height: f64) {
    if let Some(win) = app.get_webview_window("main") {
        let _ = win.set_size(tauri::Size::Logical(tauri::LogicalSize { width, height }));
    }
}
#[tauri::command]
fn move_window(app: AppHandle, x: f64, y: f64) {
    if let Some(win) = app.get_webview_window("main") {
        let _ = win.set_position(tauri::Position::Logical(tauri::LogicalPosition { x, y }));
    }
}

/// Position all 3 overlay windows in a clean layout:
///  - Bar (main):       centre-top of screen
///  - Live Answer:      directly below bar, centre
///  - Chat:             right-hand side, full-height
#[tauri::command]
fn arrange_windows(app: AppHandle) {
    // Get primary monitor size
    if let Ok(monitors) = xcap::Monitor::all() {
        if let Some(mon) = monitors.first() {
            let sw = mon.width().unwrap_or(1920) as f64;
            let _sh = mon.height().unwrap_or(1080) as f64;

            // Bar: use its current logical width, horizontal center, near top
            let mut bar_w = 700.0_f64;
            let bar_h = 35.0_f64;
            if let Some(win) = app.get_webview_window("main") {
                if let Ok(size) = win.outer_size() {
                    if let Ok(sf) = win.scale_factor() {
                        bar_w = size.to_logical::<f64>(sf).width;
                    }
                }
            }
            let bar_x = (sw - bar_w) / 2.0;
            let bar_y = 20.0_f64;

            // Live Answer: 650 x 300, directly below bar
            let ans_w = 650.0_f64;
            let ans_h = 300.0_f64;
            let ans_x = (sw - ans_w) / 2.0;
            let ans_y = bar_y + bar_h + 10.0;

            // Chat: 400 x 700, right side
            let chat_w = 400.0_f64;
            let chat_h = 700.0_f64;
            let chat_x = sw - chat_w - 10.0;
            let chat_y = bar_y;

            if let Some(win) = app.get_webview_window("main") {
                let _ = win.set_position(tauri::Position::Logical(tauri::LogicalPosition { x: bar_x, y: bar_y }));
                // Do not force the size, let resizeWindowToContent handle it
            }
            if let Some(win) = app.get_webview_window("liveAnswer") {
                let _ = win.set_position(tauri::Position::Logical(tauri::LogicalPosition { x: ans_x, y: ans_y }));
                let _ = win.set_size(tauri::Size::Logical(tauri::LogicalSize { width: ans_w, height: ans_h }));
                let _ = win.show();
            }
            if let Some(win) = app.get_webview_window("llmResponse") {
                let _ = win.set_position(tauri::Position::Logical(tauri::LogicalPosition { x: chat_x, y: chat_y }));
                let _ = win.set_size(tauri::Size::Logical(tauri::LogicalSize { width: chat_w, height: chat_h }));
                let _ = win.show();
            }
            if let Some(win) = app.get_webview_window("llmResponseWindow") {
                let llm_w = 950.0_f64;
                let llm_x = sw - llm_w - 10.0;
                let _ = win.set_position(tauri::Position::Logical(tauri::LogicalPosition { x: llm_x, y: chat_y }));
                let _ = win.set_size(tauri::Size::Logical(tauri::LogicalSize { width: llm_w, height: chat_h }));
                let _ = win.show();
            }
        }
    }
}

#[tauri::command]
fn get_session_history(state: State<'_, AppState>) -> serde_json::Value {
    let history = state.history.lock().unwrap();
    serde_json::to_value(history.clone()).unwrap_or(json!([]))
}

#[tauri::command]
fn clear_session_memory(app: AppHandle, state: State<'_, AppState>) {
    state.history.lock().unwrap().clear();
    let _ = app.emit("session-cleared", ());
}

#[tauri::command]
fn save_settings(app: tauri::AppHandle, settings: serde_json::Value) {
    use tauri::Manager;
    if let Ok(path) = app.path().app_data_dir() {
        let _ = std::fs::create_dir_all(&path);
        let settings_path = path.join("settings.json");

        let mut current_settings = if let Ok(data) = std::fs::read_to_string(&settings_path) {
            serde_json::from_str::<serde_json::Value>(&data).unwrap_or_else(|_| serde_json::json!({}))
        } else {
            serde_json::json!({})
        };

        if let (Some(current_obj), Some(new_obj)) = (current_settings.as_object_mut(), settings.as_object()) {
            for (k, v) in new_obj {
                current_obj.insert(k.clone(), v.clone());
            }
        } else {
            current_settings = settings;
        }

        if let Ok(json) = serde_json::to_string_pretty(&current_settings) {
            let _ = std::fs::write(settings_path, json);
        }
    }
}

#[tauri::command]
async fn show_settings(app: tauri::AppHandle) {
    use tauri::Manager;
    if app.get_webview_window("settings").is_none() {
        let window_result = tauri::WebviewWindowBuilder::new(
            &app,
            "settings",
            tauri::WebviewUrl::App("settings.html".into())
        )
        .title("Settings")
        .inner_size(500.0, 600.0)
        .resizable(false)
        .always_on_top(true)
        .transparent(true)
        .decorations(false)
        .content_protected(true)
        .skip_taskbar(true)
        .build();
        
        let _ = window_result;
    } else {
        if let Some(window) = app.get_webview_window("settings") {
            let _ = window.show();
            let _ = window.set_focus();
        }
    }
}
#[tauri::command] fn update_app_icon(_icon_key: String) {}

#[tauri::command]
fn update_active_skill(state: State<'_, AppState>, skill: String) {
    *state.active_skill.lock().unwrap() = skill;
}

#[tauri::command] fn restart_app_for_stealth() {}

#[tauri::command]
fn toggle_answer_hold(app: AppHandle, state: State<'_, AppState>) {
    let mut held = state.answer_held.lock().unwrap();
    *held = !*held;
    let _ = app.emit("answer-hold-toggled", json!({ "held": *held }));
}

#[tauri::command] fn copy_to_clipboard(_text: String) {}

#[tauri::command]
fn legacy_send(app: AppHandle, state: State<'_, AppState>, channel: String, data: serde_json::Value) {
    match channel.as_str() {
        "save-settings" => {
            save_settings(app, data);
        }
        "update-skill" => {
            if let Some(skill) = data.as_str() {
                update_active_skill(state, skill.to_string());
            }
        }
        "quit-app" => {
            app.exit(0);
        }
        _ => {
            println!("Unhandled legacy_send channel: {}", channel);
        }
    }
}

#[tauri::command]
fn get_desktop_audio_source() -> String { "default".into() }

#[tauri::command]
fn get_skill_prompt(_skill: String) -> String { "You are a helpful assistant.".into() }

#[tauri::command]
fn get_gemini_status() -> String { "connected".into() }

#[tauri::command]
fn test_gemini_connection() -> bool { true }

#[tauri::command]
fn notify_main_window_ready(app: tauri::AppHandle) {
    use tauri::Manager;
    if let Some(win) = app.get_webview_window("main") {
        let _ = win.show();
    }
}

#[tauri::command]
fn notify_llm_window_ready(app: tauri::AppHandle) {
    use tauri::Manager;
    if let Some(win) = app.get_webview_window("llmResponse") {
        let _ = win.show();
        let _ = win.set_focus();
    }
    if let Some(win) = app.get_webview_window("llmResponseWindow") {
        let _ = win.show();
        let _ = win.set_focus();
    }
}

#[tauri::command]
fn get_window_stats() -> serde_json::Value { serde_json::json!({}) }

fn get_chunk_rms(chunk: &[u8]) -> f64 {
    if chunk.is_empty() { return 0.0; }
    let mut sum_sq = 0.0;
    let sample_count = chunk.len() / 2;
    for i in 0..sample_count {
        let sample = i16::from_le_bytes([chunk[i * 2], chunk[i * 2 + 1]]) as f64 / 32768.0;
        sum_sq += sample * sample;
    }
    (sum_sq / sample_count as f64).sqrt()
}

#[tauri::command]
fn send_audio_chunk(app: AppHandle, state: State<'_, AppState>, chunk: Vec<u8>) {
    send_audio_chunk_internal(&app, &state, chunk);
}

fn send_audio_chunk_internal(app: &AppHandle, state: &State<'_, AppState>, chunk: Vec<u8>) {
    if !*state.speech.is_recording.lock().unwrap() { return; }
    
    if chunk.is_empty() { return; }

    let chunk_ms = chunk.len() as f64 / 32.0; 
    let energy = get_chunk_rms(&chunk);
    
    let mut noise_init = state.speech.vad_noise_init.lock().unwrap();
    let mut noise_floor = state.speech.vad_noise_floor.lock().unwrap();
    let mut speaking = state.speech.vad_speaking.lock().unwrap();
    let mut speech_ms = state.speech.vad_speech_ms.lock().unwrap();
    let mut silence_ms = state.speech.vad_silence_ms.lock().unwrap();
    let mut pre_roll = state.speech.vad_pre_roll.lock().unwrap();
    let mut pre_roll_ms = state.speech.vad_pre_roll_ms.lock().unwrap();
    let mut buffer = state.speech.segment_buffer.lock().unwrap();

    let floor_base = 0.008_f64; 
    if !*noise_init {
        *noise_floor = energy.min(floor_base);
        *noise_init = true;
    }

    let enter_threshold = floor_base.max(*noise_floor * 2.5);
    let exit_threshold = (floor_base * 0.7).max(*noise_floor * 1.6);
    let is_voiced = if *speaking { energy >= exit_threshold } else { energy >= enter_threshold };

    if !*speaking {
        if is_voiced {
            *speaking = true;
            *speech_ms = 0.0;
            *silence_ms = 0.0;
            for p in pre_roll.drain(..) {
                buffer.extend_from_slice(&p);
            }
            *pre_roll_ms = 0.0;
            buffer.extend_from_slice(&chunk);
            *speech_ms += chunk_ms;
        } else {
            *noise_floor = *noise_floor * 0.95 + energy * 0.05;
            pre_roll.push(chunk.clone());
            *pre_roll_ms += chunk_ms;
            while *pre_roll_ms > 300.0 && pre_roll.len() > 1 {
                let dropped = pre_roll.remove(0);
                *pre_roll_ms -= dropped.len() as f64 / 32.0;
            }
        }
        return;
    }

    buffer.extend_from_slice(&chunk);
    if is_voiced {
        *speech_ms += chunk_ms;
        *silence_ms = 0.0;
    } else {
        *silence_ms += chunk_ms;
    }

    let paused_long_enough = *silence_ms >= 700.0;
    let have_real_speech = *speech_ms >= 150.0;
    
    // Hard cap chunk size based on actual buffer bytes to prevent Deepgram SLOW_UPLOAD timeouts
    // 32 bytes per ms for 16kHz 1-channel 16-bit PCM. 480000 bytes = 15 seconds.
    let too_long = buffer.len() >= 480_000;

    if (paused_long_enough && have_real_speech) || too_long {
        *speaking = false;
        *speech_ms = 0.0;
        *silence_ms = 0.0;
        pre_roll.clear();
        *pre_roll_ms = 0.0;
        
        let buf_to_process = buffer.clone();
        buffer.clear();
        
        process_speech_segment(app.clone(), buf_to_process);
    } else if paused_long_enough && !have_real_speech {
        buffer.clear();
        *speaking = false;
        *speech_ms = 0.0;
        *silence_ms = 0.0;
    }
}

fn process_speech_segment(app: AppHandle, buf: Vec<u8>) {
    println!("process_speech_segment started with {} bytes", buf.len());
    tauri::async_runtime::spawn(async move {
        println!("Async task running...");
        let spec = hound::WavSpec {
            channels: 1,
            sample_rate: 16000,
            bits_per_sample: 16,
            sample_format: hound::SampleFormat::Int,
        };
        
        let mut cursor = Cursor::new(Vec::new());
        {
            let mut writer = hound::WavWriter::new(&mut cursor, spec).unwrap();
            let mut i = 0;
            while i < buf.len() {
                if i + 1 < buf.len() {
                    let sample = i16::from_le_bytes([buf[i], buf[i + 1]]);
                    writer.write_sample(sample).unwrap();
                }
                i += 2;
            }
            writer.finalize().unwrap();
        }
        let wav_data = cursor.into_inner();
        println!("WAV encoded, size: {} bytes", wav_data.len());

        let deepgram_key = get_env_or_setting(&app, "deepgramSpeechKey", "DEEPGRAM_SPEECH_KEY", "");
        let deepgram_model = get_env_or_setting(&app, "deepgramSpeechModel", "DEEPGRAM_SPEECH_MODEL", "nova-3");
        
        let groq_key = get_env_or_setting(&app, "groqSpeechKey", "GROQ_SPEECH_KEY", "");
        let groq_model = get_env_or_setting(&app, "groqSpeechModel", "GROQ_SPEECH_MODEL", "whisper-large-v3");

        let client = reqwest::Client::new();
        let mut transcription_text: Option<String> = None;
        println!("Keys loaded: Deepgram={}, Groq={}", !deepgram_key.is_empty(), !groq_key.is_empty());

        if !deepgram_key.is_empty() {
            println!("Sending to Deepgram...");
            let url = format!("https://api.deepgram.com/v1/listen?model={}&smart_format=true", deepgram_model);
            let res = client.post(&url)
                .header("Authorization", format!("Token {}", deepgram_key))
                .header("Content-Type", "audio/wav")
                .header("Content-Length", wav_data.len().to_string())
                .body(wav_data.clone())
                .timeout(std::time::Duration::from_secs(45))
                .send().await;
            println!("Deepgram response received.");
                
            match res {
                Ok(response) => {
                    if let Ok(json_res) = response.json::<serde_json::Value>().await {
                        if let Some(results) = json_res.get("results") {
                            if let Some(channels) = results.get("channels") {
                                if let Some(first_channel) = channels.get(0) {
                                    if let Some(alternatives) = first_channel.get("alternatives") {
                                        if let Some(first_alt) = alternatives.get(0) {
                                            if let Some(transcript) = first_alt.get("transcript") {
                                                if let Some(t_str) = transcript.as_str() {
                                                    transcription_text = Some(t_str.trim().to_string());
                                                }
                                            }
                                        }
                                    }
                                }
                            } else {
                                println!("Deepgram 'channels' was missing or empty.");
                            }
                        } else {
                            println!("Deepgram response did not contain 'results': {:?}", json_res);
                        }
                    } else {
                        println!("Deepgram request failed to parse JSON.");
                    }
                },
                Err(e) => println!("Deepgram HTTP request failed: {:?}", e),
            }
            println!("Deepgram processing complete. Text extracted: {:?}", transcription_text);
        }

        if transcription_text.is_none() && !groq_key.is_empty() {
            println!("Falling back to Groq for transcription...");
            let file_part = multipart::Part::bytes(wav_data)
                .file_name("audio.wav")
                .mime_str("audio/wav")
                .unwrap();
                    
            let form = multipart::Form::new()
                .part("file", file_part)
                .text("model", groq_model)
                .text("response_format", "json");

            let res = client.post("https://api.groq.com/openai/v1/audio/transcriptions")
                .header("Authorization", format!("Bearer {}", groq_key))
                .multipart(form)
                .timeout(std::time::Duration::from_secs(45))
                .send().await;
            println!("Groq response received.");
                
            match res {
                Ok(response) => {
                    if let Ok(json_res) = response.json::<serde_json::Value>().await {
                        if let Some(text) = json_res.get("text").and_then(|t| t.as_str()) {
                            transcription_text = Some(text.trim().to_string());
                        } else {
                            println!("Groq response did not contain 'text': {:?}", json_res);
                        }
                    } else {
                        println!("Groq HTTP request failed to parse JSON.");
                    }
                },
                Err(e) => println!("Groq HTTP request failed: {:?}", e),
            }
            println!("Groq processing complete. Text extracted: {:?}", transcription_text);
        }

        if let Some(text) = transcription_text {
            if !text.trim().is_empty() {
                
                let combined_text;
                {
                    let state = app.state::<AppState>();
                    let mut buf = state.transcription_buffer.lock().unwrap();
                    let mut count = state.transcription_count.lock().unwrap();
                    
                    if !buf.is_empty() {
                        buf.push_str(" ");
                    }
                    buf.push_str(text.trim());
                    *count += 1;
                    
                    let is_question = text.trim().ends_with('?');
                    println!("Sentence buffered. Count: {}/3. Is question: {}", count, is_question);
                    
                    if *count >= 3 || is_question {
                        combined_text = Some(buf.clone());
                        println!("Buffer threshold reached! Flushing to Gemini: {:?}", combined_text);
                        buf.clear();
                        *count = 0;
                    } else {
                        combined_text = None;
                    }
                }
                
                if let Some(final_text) = combined_text {
                    switch_to_chat_window(app.clone());
                    switch_to_live_answer_window(app.clone());
                    
                    // IPC Event
                    let _ = app.emit("transcription-received", serde_json::json!({ "text": final_text.clone() }));
                    
                    // Foolproof injection directly into the DOM just in case the window was still loading when IPC fired
                    if let Some(win) = app.get_webview_window("llmResponse") {
                        let escaped_json = serde_json::to_string(&final_text).unwrap_or_else(|_| "\"\"".to_string());
                        let js_code = format!(
                            r#"if (typeof window.electronAPI !== 'undefined') {{
                                setTimeout(() => {{
                                    const chatMessages = document.getElementById('chatMessages');
                                    if (chatMessages) {{
                                        // Check if this message was already added by loadHistory or IPC
                                        const existingText = {}.replace(/&/g, '&amp;').replace(/</g, '&lt;').replace(/>/g, '&gt;');
                                        const alreadyExists = Array.from(chatMessages.querySelectorAll('.message-text')).some(el => el.innerHTML.includes(existingText) || el.textContent.includes(existingText));
                                        
                                        if (!alreadyExists) {{
                                            const msgDiv = document.createElement('div');
                                            msgDiv.className = 'message transcription';
                                            
                                            const timeDiv = document.createElement('div');
                                            timeDiv.className = 'message-time';
                                            timeDiv.textContent = new Date().toLocaleTimeString();
                                            
                                            const textDiv = document.createElement('div');
                                            textDiv.className = 'message-text';
                                            textDiv.textContent = {};
                                            
                                            msgDiv.appendChild(timeDiv);
                                            msgDiv.appendChild(textDiv);
                                            
                                            chatMessages.appendChild(msgDiv);
                                            window.scrollTo(0, document.body.scrollHeight);
                                        }}
                                    }}
                                }}, 300);
                            }}"#, 
                            escaped_json, escaped_json
                        );
                        let _ = win.eval(&js_code);
                    }
                    
                    let state = app.state::<AppState>();
                    let _ = send_chat_message_internal(app.clone(), &state, final_text, true).await;
                }
            } else {
                let _ = app.emit("speech-error", serde_json::json!({ "error": "Transcription was completely empty." }));
            }
        } else {
            let err_msg = "Transcription API failed. Check your DEEPGRAM_SPEECH_KEY and GROQ_SPEECH_KEY in .env, and terminal logs for details.";
            println!("{}", err_msg);
            let _ = app.emit("speech-error", serde_json::json!({ "error": err_msg }));
        }
    });
}

#[tauri::command]
fn close_window(window: tauri::Window) {
    let _ = window.close();
}

#[tauri::command]
fn quit_app(app: AppHandle) {
    app.exit(0);
}

#[tauri::command]
async fn send_chat_message(app: AppHandle, state: State<'_, AppState>, text: String) -> Result<(), String> {
    send_chat_message_internal(app, &state, text, false).await
}

async fn send_chat_message_internal(app: AppHandle, state: &State<'_, AppState>, text: String, is_transcription: bool) -> Result<(), String> {
    {
        let mut history = state.history.lock().unwrap();
        history.push(serde_json::json!({"role": "user", "parts": [{"text": text.clone()}]}));
    }
    
    let history_clone = state.history.lock().unwrap().clone();
    let skill = state.active_skill.lock().unwrap().clone();
    let base_prompt = get_system_prompt(&skill);
    
    let prompt = if is_transcription {
        format!(
            "{}\n\nCRITICAL LIVE TRANSCRIPTION RULES:\n\
            1. You are receiving live, fragmented transcription of an interviewer speaking.\n\
            2. NEVER output conversational filler like 'Let me know when you are ready', 'Sure, I can help', or 'Here is the answer'.\n\
            3. DO NOT act like a chatbot. Act as a silent, invisible copilot.\n\
            4. If the transcribed text is a statement or conversational (not a question), respond extremely briefly or not at all.\n\
            5. Only provide your full, structured technical answer when a clear technical question or problem is presented.",
            base_prompt
        )
    } else {
        base_prompt
    };

    tauri::async_runtime::spawn(async move {
        // Live-reload .env so users don't have to restart the app
        let _ = dotenvy::dotenv_override().ok();
        let api_key = get_env_or_setting(&app, "geminiKey", "GEMINI_API_KEY", "");
        let model = get_env_or_setting(&app, "geminiModel", "GEMINI_MODEL", "gemini-3.5-flash-lite");
        
        let emit_error = {
            let app = app.clone();
            move |err_msg: &str| {
                println!("LLM Error: {}", err_msg);
                let _ = app.emit("llm-error", serde_json::json!({ "error": err_msg }));
            }
        };

        if api_key.is_empty() { 
            println!("GEMINI_API_KEY environment variable is not set!");
            emit_error("GEMINI_API_KEY is not set! Click Settings (gear icon) on the top bar and enter your Gemini API Key.");
            return; 
        }

        println!("Sending combined text to Gemini...");

        let message_id = format!("msg-{}", std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).unwrap().as_millis());
        let _ = app.emit("show-loading", serde_json::json!({
            "messageId": message_id.clone(),
            "skill": skill
        }));

        let client = reqwest::Client::builder()
            .danger_accept_invalid_certs(true)
            .user_agent("Mozilla/5.0 (Windows NT 10.0; Win64; x64) AppleWebKit/537.36 (KHTML, like Gecko) Chrome/122.0.0.0 Safari/537.36")
            .timeout(std::time::Duration::from_secs(90))
            .build()
            .unwrap_or_else(|_| reqwest::Client::new());
            
        let mut models_to_try = vec![
            model.clone(),
            "gemini-3.5-flash-lite".to_string(),
            "gemini-3.1-flash-lite".to_string(),
            "gemini-3.5-flash".to_string(),
            "gemini-flash-latest".to_string(),
            "gemini-pro-latest".to_string()
        ];
        models_to_try.dedup();

        let mut last_error_msg = String::new();

        for try_model in models_to_try {
            let url = format!("https://generativelanguage.googleapis.com/v1beta/models/{}:streamGenerateContent?alt=sse&key={}", try_model, api_key);
            
            let body = serde_json::json!({ 
                "system_instruction": {
                    "parts": [{"text": prompt.clone()}]
                },
                "contents": history_clone.clone()
            });

            println!("Gemini request sent for model {}. Waiting for response...", try_model);
            if let Ok(mut r) = client.post(&url).json(&body).send().await {
                let status = r.status().as_u16();
                println!("Gemini response connected! Status: {}", status);
                
                if status == 503 || status == 429 || status == 404 {
                    println!("API returned {} for {}. Trying fallback model...", status, try_model);
                    last_error_msg = format!("Model {} returned status {}", try_model, status);
                    tokio::time::sleep(std::time::Duration::from_millis(500)).await;
                    continue;
                }

                let mut is_retryable_error = false;
                let mut response_text = String::new();
                let mut buffer = String::new();
                let mut first_chunk_processed = false;
                let mut start_emitted = false;
                let mut start_emitted_live = false;

                while let Ok(Some(chunk)) = r.chunk().await {
                    if let Ok(text) = std::str::from_utf8(&chunk) {
                        if !first_chunk_processed {
                            first_chunk_processed = true;
                            let trimmed = text.trim_start();
                            if trimmed.starts_with('{') || trimmed.starts_with("[\n  {\n    \"error\"") {
                                if text.contains("\"code\": 503") || text.contains("\"code\": 429") || text.contains("\"code\": 404") || text.contains("\"code\":503") || text.contains("\"status\": \"UNAVAILABLE\"") || text.contains("\"status\": \"NOT_FOUND\"") {
                                    println!("API returned retryable JSON error for {}. Trying fallback...", try_model);
                                    is_retryable_error = true;
                                    break;
                                } else {
                                    emit_error(&format!("API Error for {}: {}", try_model, text));
                                    return;
                                }
                            }
                        }

                        buffer.push_str(text);
                        while let Some(pos) = buffer.find('\n') {
                            let line = buffer[..pos].to_string();
                            buffer.drain(..=pos);
                            
                            let line = line.trim();
                            if line.starts_with("data: ") {
                                let json_str = &line[6..];
                                if json_str == "[DONE]" { continue; }
                                
                                if let Ok(json_res) = serde_json::from_str::<serde_json::Value>(json_str) {
                                    if let Some(candidates) = json_res.get("candidates") {
                                        if let Some(first) = candidates.get(0) {
                                            if let Some(content) = first.get("content") {
                                                if let Some(parts) = content.get("parts") {
                                                    if let Some(part) = parts.get(0) {
                                                        if let Some(text_val) = part.get("text") {
                                                            if let Some(new_text) = text_val.as_str() {
                                                                println!("Extracted text from {}: {}", try_model, new_text);
                                                                response_text.push_str(new_text);
                                                                
                                                                if let Some(win) = app.get_webview_window("llmResponse") {
                                                                    if !start_emitted {
                                                                        let _ = win.emit("transcription-llm-response-start", serde_json::json!({
                                                                            "messageId": message_id.clone()
                                                                        }));
                                                                        start_emitted = true;
                                                                    }
                                                                    let _ = win.emit("transcription-llm-response-chunk", serde_json::json!({
                                                                        "messageId": message_id.clone(),
                                                                        "delta": new_text,
                                                                        "textSoFar": response_text.clone()
                                                                    }));
                                                                }
                                                                if is_transcription {
                                                                    if let Some(win) = app.get_webview_window("liveAnswer") {
                                                                        if !start_emitted_live {
                                                                            let _ = win.show();
                                                                            let _ = win.emit("transcription-llm-response-start", serde_json::json!({
                                                                                "messageId": message_id.clone()
                                                                            }));
                                                                            start_emitted_live = true;
                                                                        }
                                                                        let _ = win.emit("transcription-llm-response-chunk", serde_json::json!({
                                                                            "messageId": message_id.clone(),
                                                                            "delta": new_text,
                                                                            "textSoFar": response_text.clone()
                                                                        }));
                                                                    }
                                                                }
                                                            }
                                                        }
                                                    }
                                                }
                                            }
                                        }
                                    }
                                }
                            }
                        }
                    }
                }

                if !buffer.is_empty() {
                    let line = buffer.trim();
                    if line.starts_with("data: ") {
                        let json_str = &line[6..];
                        if json_str != "[DONE]" {
                            if let Ok(json_res) = serde_json::from_str::<serde_json::Value>(json_str) {
                                if let Some(candidates) = json_res.get("candidates") {
                                    if let Some(first) = candidates.get(0) {
                                        if let Some(content) = first.get("content") {
                                            if let Some(parts) = content.get("parts") {
                                                if let Some(part) = parts.get(0) {
                                                    if let Some(text_val) = part.get("text") {
                                                        if let Some(new_text) = text_val.as_str() {
                                                            println!("Extracted text (final): {}", new_text);
                                                            response_text.push_str(new_text);
                                                        }
                                                    }
                                                }
                                            }
                                        }
                                    }
                                }
                            }
                        }
                    }
                }

                if is_retryable_error {
                    last_error_msg = format!("Model {} returned retryable stream error", try_model);
                    tokio::time::sleep(std::time::Duration::from_millis(500)).await;
                    continue;
                }

                if response_text.is_empty() {
                    emit_error(&format!("Failed to parse Gemini API response JSON from {} (stream was empty).", try_model));
                } else {
                    let app_state = app.state::<AppState>();
                    app_state.history.lock().unwrap().push(serde_json::json!({"role": "model", "parts": [{"text": response_text.clone()}]}));
                    
                    if let Some(win) = app.get_webview_window("llmResponse") {
                        let _ = win.emit("transcription-llm-response", serde_json::json!({
                            "messageId": message_id.clone(),
                            "response": response_text.clone()
                        }));

                        // Foolproof injection directly into the DOM just in case IPC failed
                        let escaped_json = serde_json::to_string(&response_text).unwrap_or_else(|_| "\"\"".to_string());
                        let js_code = format!(
                            r#"if (typeof window.electronAPI !== 'undefined') {{
                                setTimeout(() => {{
                                    const chatMessages = document.getElementById('chatMessages');
                                    if (chatMessages) {{
                                        // Remove the streaming bubble if it exists
                                        const streamBubble = chatMessages.querySelector(`[data-stream-id="{}"]`);
                                        if (streamBubble) streamBubble.remove();
                                        
                                        const msgDiv = document.createElement('div');
                                        msgDiv.className = 'message assistant';
                                        
                                        const timeDiv = document.createElement('div');
                                        timeDiv.className = 'message-time';
                                        timeDiv.textContent = new Date().toLocaleTimeString();
                                        
                                        const textDiv = document.createElement('div');
                                        textDiv.className = 'message-text';
                                        
                                        if (window.marked) {{
                                            textDiv.innerHTML = marked.parse({});
                                        }} else {{
                                            textDiv.textContent = {};
                                        }}
                                        
                                        msgDiv.appendChild(timeDiv);
                                        msgDiv.appendChild(textDiv);
                                        chatMessages.appendChild(msgDiv);
                                        
                                        const indicator = document.getElementById('thinking-indicator');
                                        if (indicator) indicator.style.display = 'none';
                                        
                                        window.scrollTo(0, document.body.scrollHeight);
                                        chatMessages.scrollTop = chatMessages.scrollHeight;
                                    }}
                                }}, 300);
                            }}"#, 
                            message_id, escaped_json, escaped_json
                        );
                        let _ = win.eval(&js_code);
                    }
                    if let Some(win) = app.get_webview_window("liveAnswer") {
                        let _ = win.emit("transcription-llm-response", serde_json::json!({
                            "messageId": message_id.clone(),
                            "response": response_text.clone()
                        }));
                        // Foolproof fallback: directly inject into liveAnswer DOM
                        let escaped_live = serde_json::to_string(&response_text).unwrap_or_else(|_| "\"\"".to_string());
                        let live_js = format!(
                            r#"setTimeout(() => {{
                                const el = document.getElementById('answerContent');
                                if (el) {{
                                    const ph = document.getElementById('placeholder');
                                    if (ph) ph.style.display = 'none';
                                    if (window.marked) {{
                                        el.innerHTML = marked.parse({});
                                    }} else {{
                                        el.textContent = {};
                                    }}
                                    el.scrollTop = el.scrollHeight;
                                }}
                            }}, 200);"#,
                            escaped_live, escaped_live
                        );
                        let _ = win.eval(&live_js);
                    }
                    println!("Gemini stream completed and finalized in UI via {}.", try_model);
                }
                
                // Successfully got a response, don't try other models
                return;
            } else {
                last_error_msg = format!("Failed to connect to {}", try_model);
                println!("Network error trying model {}. Trying fallback...", try_model);
            }
        }

        // If we exhaust the loop without a successful return:
        emit_error(&format!("Failed to get a response from any Gemini model. Last error: {}", last_error_msg));

    });
    
    Ok(())
}

fn main() {
    dotenvy::dotenv().ok();
    
    let ctrl_shift_c = Shortcut::new(Some(Modifiers::CONTROL | Modifiers::SHIFT), Code::KeyC);
    let ctrl_shift_s = Shortcut::new(Some(Modifiers::CONTROL | Modifiers::SHIFT), Code::KeyS);
    let alt_a = Shortcut::new(Some(Modifiers::ALT), Code::KeyA);
    let alt_r = Shortcut::new(Some(Modifiers::ALT), Code::KeyR);
    let alt_h = Shortcut::new(Some(Modifiers::ALT), Code::KeyH);
    
    let app_state = AppState {
        history: Mutex::new(Vec::new()),
        active_skill: Mutex::new("dsa".to_string()),
        is_interactive: Mutex::new(true),
        answer_held: Mutex::new(false),
        transcription_buffer: Mutex::new(String::new()),
        transcription_count: Mutex::new(0),
        speech: SpeechState::default(),
    };

    tauri::Builder::default()
        .manage(app_state)
        .plugin(
            tauri_plugin_global_shortcut::Builder::new()
                .with_shortcuts([ctrl_shift_c, ctrl_shift_s, alt_a, alt_r, alt_h])
                .unwrap()
                .with_handler(move |app, shortcut, event| {
                    if event.state() == ShortcutState::Pressed {
                        if shortcut == &ctrl_shift_c {
                            switch_to_chat_window(app.clone());
                            switch_to_live_answer_window(app.clone());
                        } else if shortcut == &ctrl_shift_s {
                            let state = app.state::<AppState>();
                            take_screenshot(app.clone(), state);
                        } else if shortcut == &alt_a {
                            let state = app.state::<AppState>();
                            let is_interactive = *state.is_interactive.lock().unwrap();
                            if is_interactive {
                                disable_window_interaction(app.clone(), state);
                            } else {
                                enable_window_interaction(app.clone(), state);
                            }
                        } else if shortcut == &alt_r {
                            let state = app.state::<AppState>();
                            let is_recording = *state.speech.is_recording.lock().unwrap();
                            let app_clone1 = app.clone();
                            if is_recording {
                                tauri::async_runtime::spawn(async move {
                                    let state = app_clone1.state::<AppState>();
                                    let _ = stop_speech_recognition(app_clone1.clone(), state).await;
                                });
                            } else {
                                tauri::async_runtime::spawn(async move {
                                    let state = app_clone1.state::<AppState>();
                                    let _ = start_speech_recognition(app_clone1.clone(), state).await;
                                });
                            }
                        } else if shortcut == &alt_h {
                            let state = app.state::<AppState>();
                            toggle_answer_hold(app.clone(), state);
                        }
                    }
                })
                .build(),
        )
        .invoke_handler(tauri::generate_handler![
            get_speech_availability, get_settings, take_screenshot, start_speech_recognition, stop_speech_recognition,
            show_all_windows, hide_all_windows, enable_window_interaction, disable_window_interaction, switch_to_chat,
            switch_to_skills, resize_window, move_window, get_session_history, clear_session_memory, send_chat_message,
            show_settings, save_settings, update_app_icon, update_active_skill, restart_app_for_stealth, close_window, quit_app, toggle_answer_hold,
            copy_to_clipboard, legacy_send, send_audio_chunk, get_desktop_audio_source, get_skill_prompt, get_gemini_status, test_gemini_connection, notify_main_window_ready, notify_llm_window_ready, get_window_stats, arrange_windows, hide_live_answer_window, notify_live_answer_window_ready
        ])
        .run(tauri::generate_context!())
        .expect("error while running tauri application");
}

#[cfg(target_os = "windows")]
pub mod loopback {
    use cpal::traits::{DeviceTrait, HostTrait, StreamTrait};
    use tauri::{AppHandle, Manager, Emitter};
    use crate::AppState;

    pub fn start_system_audio_capture(app: AppHandle) -> Option<cpal::Stream> {
        let host = cpal::default_host();
        let device = host.default_output_device()?;
        let config = device.default_output_config().ok()?;
        
        let stream_config: cpal::StreamConfig = config.clone().into();
        let sample_rate = stream_config.sample_rate as u32;
        let channels = config.channels();
        
        let err_fn = {
            let app_clone = app.clone();
            move |err| {
                let err_str = format!("{}", err);
                if !err_str.contains("A buffer underrun or overrun occurred") {
                    eprintln!("loopback error: {}", err_str);
                    if err_str.contains("The stream configuration is no longer valid") || err_str.contains("-2004287484") {
                        let _ = app_clone.emit("restart-speech-recognition", ());
                    }
                }
            }
        };
        
        let stream = match config.sample_format() {
            cpal::SampleFormat::F32 => {
                device.build_input_stream(
                    stream_config.clone().into(),
                    move |data: &[f32], _: &cpal::InputCallbackInfo| {
                        process_samples(&app, data, sample_rate, channels);
                    },
                    err_fn,
                    None
                ).ok()?
            },
            cpal::SampleFormat::I16 => {
                device.build_input_stream(
                    stream_config.clone().into(),
                    move |data: &[i16], _: &cpal::InputCallbackInfo| {
                        let f32_data: Vec<f32> = data.iter().map(|&s| s as f32 / 32768.0).collect();
                        process_samples(&app, &f32_data, sample_rate, channels);
                    },
                    err_fn,
                    None
                ).ok()?
            },
            cpal::SampleFormat::I32 => {
                device.build_input_stream(
                    stream_config.clone().into(),
                    move |data: &[i32], _: &cpal::InputCallbackInfo| {
                        let f32_data: Vec<f32> = data.iter().map(|&s| s as f32 / 2147483648.0).collect();
                        process_samples(&app, &f32_data, sample_rate, channels);
                    },
                    err_fn,
                    None
                ).ok()?
            },
            _ => {
                println!("unsupported sample format");
                return None;
            }
        };
        
        stream.play().ok()?;
        Some(stream)
    }

    pub fn start_microphone_capture(app: AppHandle) -> Option<cpal::Stream> {
        let host = cpal::default_host();
        let device = host.default_input_device()?;
        let config = device.default_input_config().ok()?;
        
        let stream_config: cpal::StreamConfig = config.clone().into();
        let sample_rate = stream_config.sample_rate;
        let channels = config.channels();
        
        let err_fn = {
            let app_clone = app.clone();
            move |err| {
                let err_str = format!("{}", err);
                if !err_str.contains("A buffer underrun or overrun occurred") {
                    eprintln!("microphone error: {}", err_str);
                    if err_str.contains("The stream configuration is no longer valid") || err_str.contains("-2004287484") {
                        let _ = app_clone.emit("restart-speech-recognition", ());
                    }
                }
            }
        };
        
        let stream = match config.sample_format() {
            cpal::SampleFormat::F32 => {
                device.build_input_stream(
                    stream_config.clone().into(),
                    move |data: &[f32], _: &cpal::InputCallbackInfo| {
                        process_samples(&app, data, sample_rate, channels);
                    },
                    err_fn,
                    None
                ).ok()?
            },
            cpal::SampleFormat::I16 => {
                device.build_input_stream(
                    stream_config.clone().into(),
                    move |data: &[i16], _: &cpal::InputCallbackInfo| {
                        let f32_data: Vec<f32> = data.iter().map(|&s| s as f32 / 32768.0).collect();
                        process_samples(&app, &f32_data, sample_rate, channels);
                    },
                    err_fn,
                    None
                ).ok()?
            },
            cpal::SampleFormat::I32 => {
                device.build_input_stream(
                    stream_config.clone().into(),
                    move |data: &[i32], _: &cpal::InputCallbackInfo| {
                        let f32_data: Vec<f32> = data.iter().map(|&s| s as f32 / 2147483648.0).collect();
                        process_samples(&app, &f32_data, sample_rate, channels);
                    },
                    err_fn,
                    None
                ).ok()?
            },
            _ => {
                println!("unsupported sample format");
                return None;
            }
        };
        
        stream.play().ok()?;
        Some(stream)
    }

    fn process_samples(app: &AppHandle, data: &[f32], sample_rate: u32, channels: u16) {
        let mut pcm16 = Vec::new();
        let ratio = sample_rate as f32 / 16000.0;
        let mut i = 0.0;
        while (i as usize) * (channels as usize) < data.len() {
            let idx = (i as usize) * channels as usize;
            let mut sum = 0.0;
            for c in 0..channels as usize {
                if idx + c < data.len() {
                    sum += data[idx + c];
                }
            }
            let sample = sum / channels as f32;
            let s = sample.max(-1.0).min(1.0);
            let pcm = if s < 0.0 { (s * 32768.0) as i16 } else { (s * 32767.0) as i16 };
            pcm16.extend_from_slice(&pcm.to_le_bytes());
            i += ratio;
        }
        
        if !pcm16.is_empty() {
            let state = app.state::<AppState>();
            crate::send_audio_chunk_internal(app, &state, pcm16);
        }
    }
}
