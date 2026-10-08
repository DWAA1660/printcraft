//! WIA (Windows Image Acquisition) through PowerShell's `WIA.DeviceManager` COM object, so no
//! `unsafe` COM bindings are needed. A small script (below) lists the scanners or scans into a
//! folder and prints one line per result: `DEVICE<TAB>id<TAB>name`, `PAGE<TAB>path` or
//! `ERROR <hresult> <message>`. It runs with `-NoProfile -ExecutionPolicy Bypass` and without
//! a console window.

use std::sync::atomic::AtomicBool;

use crate::{Backend, ColorMode, MAX_PAGES, ScanError, ScanSettings, ScannedPage, Scanner, Source};

/// The script. WIA property ids: 3087 Document Handling Status (bit 0: paper in the feeder),
/// 3088 Document Handling Select (1 feeder, 2 flatbed, 4 duplex), 3096 Pages (0 = all),
/// 6146 Current Intent (1 colour, 2 gray, 4 black and white), 6147/6148 resolution,
/// 6151/6152 extent in pixels. The PNG format GUID is WiaFormatPNG.
pub const SCRIPT: &str = r#"param(
  [string]$Mode = 'list',
  [string]$Device = '',
  [int]$Intent = 1,
  [int]$Dpi = 300,
  [string]$Source = 'flatbed',
  [double]$WidthIn = 0,
  [double]$HeightIn = 0,
  [string]$OutDir = '',
  [int]$MaxPages = 500
)
$ErrorActionPreference = 'Stop'
function Inner($e) { $x = $e.Exception; while ($x.InnerException) { $x = $x.InnerException }; return $x }
function Fail($e) {
  $x = Inner $e
  $h = '{0:X8}' -f ($x.HResult -band 0xFFFFFFFF)
  $m = ($x.Message -replace '\s+', ' ')
  Write-Output "ERROR $h $m"
  exit 2
}
function SetProp($props, $id, $value) {
  foreach ($p in $props) { if ($p.PropertyID -eq $id) { try { $p.Value = $value } catch { }; return } }
}
function GetProp($props, $id) {
  foreach ($p in $props) { if ($p.PropertyID -eq $id) { return $p.Value } }
  return $null
}
try {
  $m = New-Object -ComObject WIA.DeviceManager
  if ($Mode -eq 'list') {
    foreach ($d in $m.DeviceInfos) {
      if ($d.Type -eq 1) {
        $n = ''
        try { $n = $d.Properties.Item('Name').Value } catch { }
        Write-Output ("DEVICE`t" + $d.DeviceID + "`t" + $n)
      }
    }
    exit 0
  }
  $info = $null
  foreach ($d in $m.DeviceInfos) { if ($d.DeviceID -eq $Device) { $info = $d } }
  if ($null -eq $info) { Write-Output 'ERROR NOTFOUND the scanner is not connected'; exit 2 }
  $dev = $info.Connect()
  $feeder = $Source -ne 'flatbed'
  if ($feeder) {
    $sel = 1
    if ($Source -eq 'duplex') { $sel = 5 }
    SetProp $dev.Properties 3088 $sel
    SetProp $dev.Properties 3096 0
  } else {
    SetProp $dev.Properties 3088 2
  }
  $item = $dev.Items.Item(1)
  SetProp $item.Properties 6146 $Intent
  SetProp $item.Properties 6147 $Dpi
  SetProp $item.Properties 6148 $Dpi
  if ($WidthIn -gt 0) { SetProp $item.Properties 6151 ([int]($WidthIn * $Dpi)) }
  if ($HeightIn -gt 0) { SetProp $item.Properties 6152 ([int]($HeightIn * $Dpi)) }
  $actual = GetProp $item.Properties 6147
  if ($null -ne $actual) { Write-Output ("DPI`t" + $actual) }
  $png = '{B96B3CAF-0728-11D3-9D7B-0000F81EF32E}'
  $n = 0
  while ($n -lt $MaxPages) {
    try {
      $img = $item.Transfer($png)
    } catch {
      $x = Inner $_
      $h = $x.HResult -band 0xFFFFFFFF
      if ($feeder -and $n -gt 0 -and $h -eq 0x80210003) { break }
      Fail $_
    }
    $n++
    $path = Join-Path $OutDir ('page{0:D4}.img' -f $n)
    $img.SaveFile($path)
    Write-Output ("PAGE`t" + $path)
    if (-not $feeder) { break }
    $status = GetProp $dev.Properties 3087
    if ($null -ne $status -and (($status -band 1) -eq 0)) { break }
  }
  exit 0
} catch { Fail $_ }
"#;

/// The script's `-Intent` for a colour mode.
pub fn intent(mode: ColorMode) -> u32 {
    match mode {
        ColorMode::Color => 1,
        ColorMode::Gray => 2,
        ColorMode::BlackWhite => 4,
    }
}

/// The script arguments for a scan into `out_dir`.
pub fn scan_args(device: &str, s: &ScanSettings, out_dir: &str) -> Vec<String> {
    let mut a = vec![
        "-Mode".to_string(),
        "scan".into(),
        "-Device".into(),
        device.to_string(),
        "-Intent".into(),
        intent(s.color).to_string(),
        "-Dpi".into(),
        s.dpi.to_string(),
        "-Source".into(),
        match s.source {
            Source::Flatbed => "flatbed",
            Source::Feeder => "feeder",
            Source::FeederDuplex => "duplex",
        }
        .into(),
        "-OutDir".into(),
        out_dir.to_string(),
        "-MaxPages".into(),
        MAX_PAGES.to_string(),
    ];
    if let Some((w, h)) = s.paper.size_mm() {
        a.extend(["-WidthIn".into(), format!("{:.3}", w / 25.4), "-HeightIn".into(), format!("{:.3}", h / 25.4)]);
    }
    a
}

/// Scanners from the script's `list` output.
pub fn parse_list(out: &str) -> Vec<Scanner> {
    out.lines()
        .filter_map(|l| {
            let mut f = l.trim_end_matches('\r').strip_prefix("DEVICE\t")?.splitn(2, '\t');
            let id = f.next()?.trim();
            if id.is_empty() {
                return None;
            }
            let name = f.next().map(crate::tidy).filter(|n| !n.is_empty()).unwrap_or_else(|| id.to_string());
            Some(Scanner { id: format!("{}{id}", Backend::Wia.prefix()), name, backend: Backend::Wia })
        })
        .take(256)
        .collect()
}

/// The error a WIA HRESULT stands for (`WIA_ERROR_*`).
pub fn error_from_hresult(code: &str, message: &str) -> ScanError {
    match code.to_ascii_uppercase().as_str() {
        "80210003" => ScanError::FeederEmpty,
        "80210002" => ScanError::Jammed,
        "80210006" => ScanError::Busy,
        "80210016" => ScanError::CoverOpen,
        "80210005" | "80210009" | "80210015" | "NOTFOUND" => ScanError::NotFound(crate::tidy(message)),
        "80210064" | "800704C7" => ScanError::Cancelled,
        _ => ScanError::Failed(crate::tidy(if message.trim().is_empty() { code } else { message })),
    }
}

/// The page files and the resolution the device accepted, from the script's `scan` output,
/// or the error it reported.
pub fn parse_scan(out: &str) -> Result<(Vec<String>, Option<u32>), ScanError> {
    let mut pages = Vec::new();
    let mut dpi = None;
    for l in out.lines().map(|l| l.trim_end_matches('\r')) {
        if let Some(d) = l.strip_prefix("DPI\t") {
            dpi = d.trim().parse::<u32>().ok().filter(|d| crate::DPI_RANGE.contains(d)).or(dpi);
        } else if let Some(p) = l.strip_prefix("PAGE\t") {
            pages.push(p.to_string());
        } else if let Some(rest) = l.strip_prefix("ERROR ") {
            let (code, msg) = rest.split_once(' ').unwrap_or((rest, ""));
            return Err(error_from_hresult(code, msg));
        }
    }
    Ok((pages, dpi))
}

#[cfg(windows)]
mod run {
    use super::*;
    use std::os::windows::process::CommandExt;

    /// CREATE_NO_WINDOW: no console flashes up from the windowed app.
    const CREATE_NO_WINDOW: u32 = 0x0800_0000;

    pub(super) fn powershell(args: &[String], dir: &std::path::Path, cancel: &AtomicBool) -> Result<String, ScanError> {
        use std::sync::atomic::Ordering;
        let script = dir.join("pdfcraft-wia.ps1");
        std::fs::write(&script, SCRIPT).map_err(|e| ScanError::Failed(e.to_string()))?;
        let mut c = std::process::Command::new("powershell.exe");
        c.args(["-NoProfile", "-NonInteractive", "-Sta", "-ExecutionPolicy", "Bypass", "-File"])
            .arg(&script)
            .args(args)
            .creation_flags(CREATE_NO_WINDOW)
            .stdin(std::process::Stdio::null())
            .stdout(std::process::Stdio::piped())
            .stderr(std::process::Stdio::piped());
        let mut child = c.spawn().map_err(|e| {
            if e.kind() == std::io::ErrorKind::NotFound {
                ScanError::BackendMissing("WIA (Windows PowerShell)")
            } else {
                ScanError::Failed(e.to_string())
            }
        })?;
        let out = child.stdout.take().map(|mut o| {
            std::thread::spawn(move || {
                use std::io::Read;
                let mut buf = Vec::new();
                let _ = o.by_ref().take(1 << 20).read_to_end(&mut buf);
                let _ = std::io::copy(&mut o, &mut std::io::sink());
                String::from_utf8_lossy(&buf).into_owned()
            })
        });
        let err = child.stderr.take().map(|mut e| {
            std::thread::spawn(move || {
                let _ = std::io::copy(&mut e, &mut std::io::sink());
            })
        });
        loop {
            if cancel.load(Ordering::Relaxed) {
                let _ = child.kill();
                let _ = child.wait();
                return Err(ScanError::Cancelled);
            }
            match child.try_wait() {
                Ok(Some(_)) => break,
                Ok(None) => std::thread::sleep(std::time::Duration::from_millis(50)),
                Err(e) => return Err(ScanError::Failed(e.to_string())),
            }
        }
        if let Some(h) = err {
            let _ = h.join();
        }
        Ok(out.and_then(|h| h.join().ok()).unwrap_or_default())
    }

    pub(super) fn temp_dir() -> Result<std::path::PathBuf, ScanError> {
        let dir = std::env::temp_dir().join(format!(
            "pdfcraft-wia-{}-{}",
            std::process::id(),
            std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map_or(0, |d| d.as_nanos())
        ));
        std::fs::create_dir_all(&dir).map_err(|e| ScanError::Failed(format!("couldn't make a folder for the pages: {e}")))?;
        Ok(dir)
    }
}

/// WIA scanners (Windows only; elsewhere none).
pub fn scanners() -> Vec<Scanner> {
    #[cfg(windows)]
    {
        let Ok(dir) = run::temp_dir() else { return Vec::new() };
        let out = run::powershell(&["-Mode".into(), "list".into()], &dir, &AtomicBool::new(false));
        let _ = std::fs::remove_dir_all(&dir);
        out.map(|o| parse_list(&o)).unwrap_or_default()
    }
    #[cfg(not(windows))]
    {
        Vec::new()
    }
}

/// Scan from WIA device `device`.
pub fn scan(device: &str, s: &ScanSettings, cancel: &AtomicBool) -> Result<Vec<ScannedPage>, ScanError> {
    #[cfg(windows)]
    {
        let dir = run::temp_dir()?;
        let result = (|| {
            let out = run::powershell(&scan_args(device, s, &dir.to_string_lossy()), &dir, cancel)?;
            let (files, dpi) = parse_scan(&out)?;
            let dpi = dpi.unwrap_or(s.dpi) as f64;
            let mut pages = Vec::new();
            for p in files.into_iter().take(MAX_PAGES) {
                let path = std::path::Path::new(&p);
                // Only files the script wrote into our folder.
                if !path.starts_with(&dir) {
                    continue;
                }
                let len = std::fs::metadata(path).map(|m| m.len()).unwrap_or(0);
                if len == 0 || len > crate::MAX_PAGE_BYTES {
                    continue;
                }
                if let Ok(bytes) = std::fs::read(path) {
                    pages.push(ScannedPage { bytes, dpi });
                }
            }
            if pages.is_empty() { Err(if s.source.is_feeder() { ScanError::FeederEmpty } else { ScanError::NoPages }) } else { Ok(pages) }
        })();
        let _ = std::fs::remove_dir_all(&dir);
        result
    }
    #[cfg(not(windows))]
    {
        let _ = (device, s, cancel);
        Err(ScanError::BackendMissing("WIA"))
    }
}
