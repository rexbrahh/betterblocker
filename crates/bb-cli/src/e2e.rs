use std::path::PathBuf;
use std::time::Duration;

use serde_json::Value;
use thirtyfour::extensions::cdp::ChromeDevTools;
use thirtyfour::prelude::*;
use thirtyfour::ChromeCapabilities;

pub struct E2eOptions {
    pub chromedriver_url: String,
    pub extension_path: String,
    pub headless: bool,
}

pub fn run_e2e(opts: E2eOptions) -> Result<(), String> {
    let runtime = tokio::runtime::Runtime::new()
        .map_err(|e| format!("Failed to start tokio runtime: {}", e))?;
    runtime.block_on(run_e2e_async(opts))
}

async fn run_e2e_async(opts: E2eOptions) -> Result<(), String> {
    let extension_path = canonicalize_path(&opts.extension_path)?;

    let mut caps = ChromeCapabilities::new();
    let disable_arg = format!("--disable-extensions-except={}", extension_path.display());
    caps.add_arg(&disable_arg)
        .map_err(|e| format!("Failed to set chrome arg: {}", e))?;
    let load_arg = format!("--load-extension={}", extension_path.display());
    caps.add_arg(&load_arg)
        .map_err(|e| format!("Failed to set chrome arg: {}", e))?;
    caps.add_arg("--no-first-run")
        .map_err(|e| format!("Failed to set chrome arg: {}", e))?;
    caps.add_arg("--no-default-browser-check")
        .map_err(|e| format!("Failed to set chrome arg: {}", e))?;
    caps.add_arg("--disable-default-apps")
        .map_err(|e| format!("Failed to set chrome arg: {}", e))?;
    if opts.headless {
        caps.add_arg("--headless=new")
            .map_err(|e| format!("Failed to set chrome arg: {}", e))?;
        caps.add_arg("--disable-gpu")
            .map_err(|e| format!("Failed to set chrome arg: {}", e))?;
    }

    let driver = WebDriver::new(&opts.chromedriver_url, caps)
        .await
        .map_err(|e| format!("Failed to connect to chromedriver: {}", e))?;

    let cdp = ChromeDevTools::new(driver.handle.clone());
    tokio::time::sleep(Duration::from_secs(1)).await;

    let extension_id = find_extension_id(&cdp)
        .await
        .ok_or_else(|| "Failed to locate extension background page".to_string())?;

    let mut errors = Vec::new();

    if let Err(e) = check_page_has_selector(
        &driver,
        &format!("chrome-extension://{}/popup/popup.html", extension_id),
        "#blocked-count",
    )
    .await
    {
        errors.push(format!("Popup page check failed: {}", e));
    }

    if let Err(e) = check_page_has_selector(
        &driver,
        &format!("chrome-extension://{}/options/options.html", extension_id),
        "#lists-container",
    )
    .await
    {
        errors.push(format!("Options page check failed: {}", e));
    }

    if let Err(e) = check_background_wasm(&driver, &extension_id).await {
        errors.push(format!("Background wasm check failed: {}", e));
    }

    if let Err(e) = check_content_script(&driver).await {
        errors.push(format!("Content script check failed: {}", e));
    }

    if let Err(e) = check_blocking_rules(&driver, &extension_id).await {
        errors.push(format!("Blocking checks failed: {}", e));
    }

    driver.quit().await.ok();

    if errors.is_empty() {
        println!("✓ E2E checks passed");
        Ok(())
    } else {
        Err(format!("E2E failed:\n- {}", errors.join("\n- ")))
    }
}

async fn find_extension_id(cdp: &ChromeDevTools) -> Option<String> {
    let targets = cdp.execute_cdp("Target.getTargets").await.ok()?;
    let infos = targets.get("targetInfos")?.as_array()?;
    for info in infos {
        let target_type = info.get("type").and_then(Value::as_str).unwrap_or("");
        let url = info.get("url").and_then(Value::as_str).unwrap_or("");
        if target_type == "background_page" && url.starts_with("chrome-extension://") {
            let id = url.trim_start_matches("chrome-extension://");
            if let Some(id) = id.split('/').next() {
                if !id.is_empty() {
                    return Some(id.to_string());
                }
            }
        }
    }
    None
}

async fn check_page_has_selector(driver: &WebDriver, url: &str, selector: &str) -> WebDriverResult<()> {
    driver.goto(url).await?;
    driver.find(By::Css(selector)).await?;
    Ok(())
}

async fn check_background_wasm(driver: &WebDriver, extension_id: &str) -> Result<(), String> {
    let url = format!("chrome-extension://{}/_generated_background_page.html", extension_id);
    driver.goto(&url).await.map_err(|e| format!("Failed to open background page: {}", e))?;
    let initialized = eval_bool(driver, "return window.wasm?.is_initialized?.() ?? false;")
        .await
        .map_err(|e| format!("Failed to read wasm state: {}", e))?;
    if !initialized {
        return Err("WASM module not initialized".to_string());
    }
    Ok(())
}

async fn check_blocking_rules(driver: &WebDriver, extension_id: &str) -> Result<(), String> {
    let url = format!("chrome-extension://{}/_generated_background_page.html", extension_id);
    driver.goto(&url).await.map_err(|e| format!("Failed to open background page: {}", e))?;

    let blocked = eval_bool(
        driver,
        "return window.wasm?.should_block?.('https://pagead2.googlesyndication.com/pagead/js/adsbygoogle.js', 'script', 'https://example.com') ?? false;",
    )
    .await
    .map_err(|e| format!("Failed to evaluate blocking rule: {}", e))?;

    if !blocked {
        return Err("Expected ad request to be blocked".to_string());
    }

    let allowed = eval_bool(
        driver,
        "return window.wasm?.should_block?.('https://example.com/', 'document', undefined) ?? false;",
    )
    .await
    .map_err(|e| format!("Failed to evaluate allow rule: {}", e))?;

    if allowed {
        return Err("Expected non-ad request to be allowed".to_string());
    }

    Ok(())
}

async fn check_content_script(driver: &WebDriver) -> Result<(), String> {
    driver.goto("https://example.com")
        .await
        .map_err(|e| format!("Failed to navigate to example.com: {}", e))?;
    let injected = eval_bool(driver, "return document.documentElement.dataset.bbInjected === '1';")
        .await
        .map_err(|e| format!("Failed to read injected flag: {}", e))?;
    if !injected {
        return Err("Content script did not inject".to_string());
    }
    Ok(())
}

async fn eval_bool(driver: &WebDriver, script: &str) -> WebDriverResult<bool> {
    let result = driver.execute(script, Vec::<Value>::new()).await?;
    Ok(result.json().as_bool().unwrap_or(false))
}

fn canonicalize_path(path: &str) -> Result<PathBuf, String> {
    std::fs::canonicalize(path)
        .map_err(|e| format!("Failed to resolve '{}': {}", path, e))
}
