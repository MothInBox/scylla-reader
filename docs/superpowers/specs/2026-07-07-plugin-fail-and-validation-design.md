# Plugin Fail Function and Host Validation

## Summary

Add two layers of validation for scraped data: a `fail()` host function that plugins
call to signal errors, and host-side structural validation as a safety net. Also
expand the detail shown when a job is expanded in the jobs UI.

## Changes

### 1. `scylla_fail` host function

Registered in `ScraperRegistry::call_cached_plugin` alongside `curl_fetch`.

The plugin calls `scylla_fail("reason")` → host reads the string from plugin
memory, logs it at DEBUG level, and returns `Err(extism::Error::msg(...))`. This
immediately terminates the Extism call. The error propagates through
`scrape_url`/`scrape_chapter` → job enters `Failed(reason)` state.

### 2. Host-side validation

After `serde_json::from_slice(&output_bytes)` in `scrape_url`:
- Reject if `title` is empty → log debug, return `Err`
- Reject if `url` is empty → log debug, return `Err`

After `serde_json::from_slice(&output_bytes)` in `scrape_chapter`:
- Reject if `title` is empty → log debug, return `Err`
- Reject if `content` is empty → log debug, return `Err`

### 3. Expanded job detail

Remove the 60-character truncation on error messages. Show up to 10 lines of
the full error text in the expanded job detail panel.

## Files
- `scrapers/services.rs` — host function + validation
- `ui/jobs.rs` — expanded error display
