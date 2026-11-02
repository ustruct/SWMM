use std::{
    io::{BufWriter, Seek, SeekFrom, Write},
    path::Path,
    sync::atomic::{AtomicU64, Ordering},
    time::Duration,
};

use anyhow::{Context, Result};

const USER_AGENT: &str = concat!("SWMM/", env!("CARGO_PKG_VERSION"));
const SEGMENT_MIN: u64 = 4 * 1024 * 1024;
const MAX_SEGMENTS: u64 = 16;

struct CountingWriter<'a, W: Write> {
    inner: W,
    done: &'a AtomicU64,
    written: u64,
    limit: Option<u64>,
}

impl<W: Write> Write for CountingWriter<'_, W> {
    fn write(&mut self, buf: &[u8]) -> std::io::Result<usize> {
        let allowed = match self.limit {
            Some(limit) => buf.len().min(limit.saturating_sub(self.written) as usize),
            None => buf.len(),
        };
        if allowed == 0 {
            return Ok(buf.len());
        }
        let written = self.inner.write(&buf[..allowed])?;
        self.written += written as u64;
        self.done.fetch_add(written as u64, Ordering::Relaxed);
        Ok(if written == allowed { buf.len() } else { written })
    }

    fn flush(&mut self) -> std::io::Result<()> {
        self.inner.flush()
    }
}

struct Target {
    url: String,
    total: Option<u64>,
    ranges: bool,
}

fn absolute(base: &str, location: &str) -> String {
    if location.starts_with("http://") || location.starts_with("https://") {
        return location.to_string();
    }
    let scheme_end = base.find("://").map(|i| i + 3).unwrap_or(0);
    let host_end = base[scheme_end..].find('/').map(|i| i + scheme_end).unwrap_or(base.len());
    if location.starts_with('/') {
        format!("{}{}", &base[..host_end], location)
    } else {
        let dir_end = base.rfind('/').filter(|i| *i >= host_end).unwrap_or(host_end);
        format!("{}/{}", &base[..dir_end], location)
    }
}

fn resolve(url: &str) -> Result<Target> {
    let mut current = url.to_string();
    for _ in 0..10 {
        let uri = http_req::uri::Uri::try_from(current.as_str())?;
        let mut sink = Vec::new();
        let response = http_req::request::Request::new(&uri)
            .header("User-Agent", USER_AGENT)
            .method(http_req::request::Method::HEAD)
            .redirect_policy(http_req::request::RedirectPolicy::<fn() -> bool>::Limit(0))
            .connect_timeout(Some(Duration::from_secs(20)))
            .read_timeout(Some(Duration::from_secs(30)))
            .timeout(Duration::from_secs(60))
            .send(&mut sink)?;
        let status = response.status_code();
        if status.is_redirect() {
            let location = response
                .headers()
                .get("Location")
                .context("Redirect without a location")?
                .to_string();
            current = absolute(&current, &location);
            continue;
        }
        if !status.is_success() {
            anyhow::bail!("The server answered with HTTP {}", u16::from(status));
        }
        let ranges = response
            .headers()
            .get("Accept-Ranges")
            .map(|v| v.to_ascii_lowercase().contains("bytes"))
            .unwrap_or(false);
        return Ok(Target {
            url: current,
            total: response.content_len().map(|l| l as u64),
            ranges,
        });
    }
    anyhow::bail!("Too many redirects")
}

fn fetch(url: &str, range: Option<(u64, u64)>, writer: &mut impl Write) -> Result<u16> {
    let uri = http_req::uri::Uri::try_from(url)?;
    let range_header = range.map(|(start, end)| format!("bytes={start}-{end}"));
    let mut request = http_req::request::Request::new(&uri);
    request
        .header("User-Agent", USER_AGENT)
        .method(http_req::request::Method::GET)
        .connect_timeout(Some(Duration::from_secs(20)))
        .read_timeout(Some(Duration::from_secs(60)))
        .timeout(Duration::from_secs(6 * 60 * 60));
    if let Some(value) = range_header.as_ref() {
        request.header("Range", value);
    }
    let response = request.send(writer)?;
    Ok(u16::from(response.status_code()))
}

fn download_segment(url: &str, dest: &Path, start: u64, end: u64, done: &AtomicU64) -> Result<()> {
    let length = end - start + 1;
    let mut got = 0u64;
    let mut last_error = None;
    for _ in 0..4 {
        if got >= length {
            return Ok(());
        }
        let mut file = std::fs::OpenOptions::new().write(true).open(dest)?;
        file.seek(SeekFrom::Start(start + got))?;
        let mut writer = CountingWriter {
            inner: BufWriter::with_capacity(1 << 20, file),
            done,
            written: 0,
            limit: Some(length - got),
        };
        let result = fetch(url, Some((start + got, end)), &mut writer);
        writer.flush()?;
        got += writer.written;
        match result {
            Ok(206) if got >= length => return Ok(()),
            Ok(206) => last_error = Some(anyhow::anyhow!("Connection closed early")),
            Ok(code) => anyhow::bail!("Range not supported (HTTP {code})"),
            Err(e) => last_error = Some(e),
        }
    }
    Err(last_error.unwrap_or_else(|| anyhow::anyhow!("Segment failed")))
}

fn download_parallel(target: &Target, total: u64, dest: &Path, done: &AtomicU64) -> Result<()> {
    std::fs::File::create(dest)?.set_len(total)?;
    let segments = (total / SEGMENT_MIN).clamp(2, MAX_SEGMENTS);
    let size = total.div_ceil(segments);
    log::info!("Downloading in {segments} parts from {}", target.url);
    std::thread::scope(|scope| {
        let handles: Vec<_> = (0..segments)
            .map(|i| {
                let start = i * size;
                let end = ((i + 1) * size).min(total) - 1;
                scope.spawn(move || download_segment(&target.url, dest, start, end, done))
            })
            .collect();
        handles
            .into_iter()
            .map(|h| h.join().unwrap_or_else(|_| Err(anyhow::anyhow!("Download thread crashed"))))
            .collect::<Result<Vec<_>>>()
            .map(|_| ())
    })
}

fn download_single(url: &str, dest: &Path, done: &AtomicU64) -> Result<()> {
    done.store(0, Ordering::Relaxed);
    let file = std::fs::File::create(dest).with_context(|| format!("Could not create {}", dest.display()))?;
    let mut writer = CountingWriter {
        inner: BufWriter::with_capacity(1 << 20, file),
        done,
        written: 0,
        limit: None,
    };
    let code = fetch(url, None, &mut writer).context("Download failed")?;
    writer.flush()?;
    if !(200..300).contains(&code) {
        anyhow::bail!("The server answered with HTTP {code}");
    }
    Ok(())
}

fn curl_program() -> Option<std::path::PathBuf> {
    let mut candidates = Vec::new();
    if let Some(root) = std::env::var_os("SystemRoot") {
        candidates.push(std::path::PathBuf::from(root).join("System32").join("curl.exe"));
    }
    candidates.push(std::path::PathBuf::from("curl"));
    candidates.into_iter().find(|program| {
        let mut command = std::process::Command::new(program);
        command
            .arg("--version")
            .stdin(std::process::Stdio::null())
            .stdout(std::process::Stdio::null())
            .stderr(std::process::Stdio::null());
        hide_window(&mut command);
        command.status().map(|s| s.success()).unwrap_or(false)
    })
}

fn hide_window(command: &mut std::process::Command) {
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        command.creation_flags(0x0800_0000);
    }
    #[cfg(not(windows))]
    let _ = command;
}

fn curl_spawn(curl: &Path, url: &str, range: Option<(u64, u64)>, dest: &Path) -> std::io::Result<std::process::Child> {
    let mut command = std::process::Command::new(curl);
    command.args([
        "--location",
        "--fail",
        "--silent",
        "--show-error",
        "--retry",
        "3",
        "--connect-timeout",
        "20",
        "--speed-limit",
        "1024",
        "--speed-time",
        "60",
        "--user-agent",
        USER_AGENT,
    ]);
    if let Some((start, end)) = range {
        command.arg("--range").arg(format!("{start}-{end}"));
    }
    command
        .arg("--output")
        .arg(dest)
        .arg(url)
        .stdin(std::process::Stdio::null())
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::piped());
    hide_window(&mut command);
    command.spawn()
}

fn file_len(path: &Path) -> u64 {
    std::fs::metadata(path).map(|m| m.len()).unwrap_or(0)
}

fn curl_error(child: std::process::Child) -> anyhow::Error {
    let output = child.wait_with_output().ok();
    let text = output
        .map(|o| String::from_utf8_lossy(&o.stderr).trim().to_string())
        .filter(|t| !t.is_empty())
        .unwrap_or_else(|| "curl failed".to_string());
    anyhow::anyhow!(text)
}

fn curl_probe(curl: &Path, url: &str) -> Option<Target> {
    let scratch = std::env::temp_dir().join(format!("swmm-probe-{}", std::process::id()));
    let mut command = std::process::Command::new(curl);
    command
        .args([
            "--location",
            "--silent",
            "--range",
            "0-0",
            "--max-filesize",
            "65536",
            "--max-time",
            "30",
            "--connect-timeout",
            "15",
            "--user-agent",
            USER_AGENT,
            "--dump-header",
            "-",
            "--write-out",
            "\nSWMM-URL:%{url_effective}\nSWMM-CODE:%{http_code}\n",
            "--output",
        ])
        .arg(&scratch)
        .arg(url)
        .stdin(std::process::Stdio::null())
        .stderr(std::process::Stdio::null());
    hide_window(&mut command);
    let output = command.output().ok()?;
    let _ = std::fs::remove_file(&scratch);
    let text = String::from_utf8_lossy(&output.stdout).replace('\r', "");
    let final_url = text.lines().find_map(|l| l.strip_prefix("SWMM-URL:"))?.trim().to_string();
    let code: u16 = text.lines().find_map(|l| l.strip_prefix("SWMM-CODE:"))?.trim().parse().ok()?;
    let last_block = text
        .split("\n\n")
        .filter(|block| block.trim_start().starts_with("HTTP/"))
        .last()
        .unwrap_or("");
    let header = |name: &str| {
        last_block.lines().find_map(|line| {
            let (key, value) = line.split_once(':')?;
            key.trim().eq_ignore_ascii_case(name).then(|| value.trim().to_string())
        })
    };
    match code {
        206 => {
            let total = header("Content-Range")
                .and_then(|range| range.rsplit('/').next().and_then(|t| t.trim().parse::<u64>().ok()));
            Some(Target {
                url: final_url,
                total,
                ranges: total.is_some(),
            })
        }
        200..=299 => Some(Target {
            url: final_url,
            total: header("Content-Length").and_then(|l| l.parse().ok()),
            ranges: false,
        }),
        _ => None,
    }
}

fn curl_single(curl: &Path, url: &str, dest: &Path, done: &AtomicU64) -> Result<()> {
    let _ = std::fs::remove_file(dest);
    let mut child = curl_spawn(curl, url, None, dest)?;
    loop {
        done.store(file_len(dest), Ordering::Relaxed);
        if let Some(status) = child.try_wait()? {
            done.store(file_len(dest), Ordering::Relaxed);
            if status.success() {
                return Ok(());
            }
            return Err(curl_error(child));
        }
        std::thread::sleep(Duration::from_millis(120));
    }
}

fn curl_parallel(curl: &Path, url: &str, total: u64, dest: &Path, done: &AtomicU64) -> Result<()> {
    let segments = (total / SEGMENT_MIN).clamp(2, MAX_SEGMENTS);
    let size = total.div_ceil(segments);
    let parts: Vec<(std::path::PathBuf, u64, u64)> = (0..segments)
        .map(|i| {
            let start = i * size;
            let end = ((i + 1) * size).min(total) - 1;
            (dest.with_extension(format!("part{i}")), start, end)
        })
        .collect();
    log::info!("Downloading in {segments} parts from {url}");
    let cleanup = |parts: &[(std::path::PathBuf, u64, u64)]| {
        for (path, ..) in parts {
            let _ = std::fs::remove_file(path);
        }
    };
    let mut children: Vec<Option<std::process::Child>> = Vec::new();
    let mut attempts = vec![0u32; parts.len()];
    for (path, start, end) in &parts {
        let _ = std::fs::remove_file(path);
        children.push(Some(curl_spawn(curl, url, Some((*start, *end)), path)?));
    }
    loop {
        let progress: u64 = parts.iter().map(|(path, ..)| file_len(path)).sum();
        done.store(progress.min(total), Ordering::Relaxed);
        let mut running = false;
        let mut failure: Option<anyhow::Error> = None;
        for i in 0..children.len() {
            let finished = match children[i].as_mut() {
                None => continue,
                Some(child) => child.try_wait()?,
            };
            let Some(status) = finished else {
                running = true;
                continue;
            };
            let (path, start, end) = &parts[i];
            let expected = end - start + 1;
            let got = file_len(path);
            if status.success() && got == expected {
                children[i] = None;
            } else if got > expected {
                if let Some(child) = children[i].take() {
                    let _ = child.wait_with_output();
                }
                failure = Some(anyhow::anyhow!("The server doesn't support split downloads"));
                break;
            } else if attempts[i] < 3 {
                attempts[i] += 1;
                if let Some(child) = children[i].take() {
                    let _ = child.wait_with_output();
                }
                let _ = std::fs::remove_file(path);
                children[i] = Some(curl_spawn(curl, url, Some((*start, *end)), path)?);
                running = true;
            } else {
                failure = Some(
                    children[i]
                        .take()
                        .map(curl_error)
                        .unwrap_or_else(|| anyhow::anyhow!("Part failed")),
                );
                break;
            }
        }
        if let Some(error) = failure {
            for child in children.iter_mut().flatten() {
                let _ = child.kill();
                let _ = child.wait();
            }
            cleanup(&parts[..]);
            return Err(error);
        }
        if !running && children.iter().all(|c| c.is_none()) {
            break;
        }
        std::thread::sleep(Duration::from_millis(120));
    }
    let mut out = BufWriter::with_capacity(4 << 20, std::fs::File::create(dest)?);
    for (path, ..) in &parts {
        let mut part = std::fs::File::open(path)?;
        std::io::copy(&mut part, &mut out)?;
    }
    out.flush()?;
    cleanup(&parts[..]);
    done.store(total, Ordering::Relaxed);
    Ok(())
}

pub fn download_file(url: &str, dest: &Path, done: &AtomicU64) -> Result<()> {
    done.store(0, Ordering::Relaxed);
    let curl = curl_program();
    let probed = curl.as_deref().and_then(|curl| curl_probe(curl, url));
    if let Some(target) = probed.as_ref() {
        log::info!(
            "Download target {} ({:?} bytes, split downloads {})",
            target.url,
            target.total,
            if target.ranges { "supported" } else { "not supported" }
        );
    }
    let target = probed.map(Ok).unwrap_or_else(|| resolve(url)).unwrap_or_else(|e| {
        log::warn!("Could not inspect {url} ({e:?}), downloading directly");
        Target {
            url: url.to_string(),
            total: None,
            ranges: false,
        }
    });
    let split = target.total.filter(|t| target.ranges && *t >= SEGMENT_MIN * 2);

    if let Some(curl) = curl {
        if let Some(total) = split {
            match curl_parallel(&curl, &target.url, total, dest, done) {
                Ok(()) => return Ok(()),
                Err(e) => log::warn!("Split download failed ({e:?}), trying one connection"),
            }
        }
        match curl_single(&curl, &target.url, dest, done) {
            Ok(()) => return Ok(()),
            Err(e) => log::warn!("curl download failed ({e:?}), using the built-in downloader"),
        }
    }

    if let Some(total) = split {
        match download_parallel(&target, total, dest, done) {
            Ok(()) => return Ok(()),
            Err(e) => log::warn!("Parallel download failed ({e:?}), downloading in one piece"),
        }
    }
    download_single(&target.url, dest, done)
}

pub fn response(url: &str) -> Result<Vec<u8>> {
    let url = url.try_into()?;
    let mut buf = Vec::new();
    http_req::request::Request::new(&url)
        .header("User-Agent", USER_AGENT)
        .method(http_req::request::Method::GET)
        .send(&mut buf)
        .context("HTTP request file")
        .and_then(|res| {
            if let Some(url) = res
                .status_code()
                .is_redirect()
                .then(|| res.headers().get("Location"))
                .flatten()
            {
                response(url)
            } else {
                Ok(buf)
            }
        })
}

pub struct SmartStringWrapper<'a>(pub &'a mut smartstring::alias::String);

impl swmm_widgets::egui::TextBuffer for SmartStringWrapper<'_> {
    #[inline]
    fn as_str(&self) -> &str {
        self.0.as_str()
    }

    fn is_mutable(&self) -> bool {
        true
    }

    #[inline]
    fn insert_text(&mut self, text: &str, char_index: usize) -> usize {
        let index = self.byte_index_from_char_index(char_index);
        self.0.insert_str(index, text);
        text.chars().count()
    }

    #[inline]
    fn delete_char_range(&mut self, char_range: std::ops::Range<usize>) {
        assert!(char_range.start <= char_range.end);
        let start = self.byte_index_from_char_index(char_range.start);
        let end = self.byte_index_from_char_index(char_range.end);
        self.0.drain(start..end);
    }
}
