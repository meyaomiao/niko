// 易支付收银台：易支付要求把下单参数以表单 POST 提交到收银台地址，
// 无法用 open(url) 打开，因此在应用内开一个独立窗口加载本地自动提交页。
use tauri::{Manager, WebviewUrl, WebviewWindowBuilder};

const CASHIER_LABEL: &str = "cashier";

fn to_hex(data: &[u8]) -> String {
    let mut out = String::with_capacity(data.len() * 2);
    for b in data {
        out.push_str(&format!("{:02x}", b));
    }
    out
}

#[tauri::command]
pub async fn open_cashier(
    app: tauri::AppHandle,
    url: String,
    params: serde_json::Value,
) -> Result<(), String> {
    if let Some(win) = app.get_webview_window(CASHIER_LABEL) {
        let _ = win.close();
    }
    let payload = serde_json::json!({ "url": url, "params": params });
    let encoded = to_hex(payload.to_string().as_bytes());
    WebviewWindowBuilder::new(
        &app,
        CASHIER_LABEL,
        WebviewUrl::App(format!("epay-redirect.html?d={}", encoded).into()),
    )
    .title("支付")
    .inner_size(520.0, 720.0)
    .resizable(true)
    .build()
    .map_err(|e| e.to_string())?;
    Ok(())
}

#[tauri::command]
pub async fn close_cashier(app: tauri::AppHandle) -> Result<(), String> {
    if let Some(win) = app.get_webview_window(CASHIER_LABEL) {
        win.close().map_err(|e| e.to_string())?;
    }
    Ok(())
}
