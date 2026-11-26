//! GitHub 在线更新（CDN 加速）
//!
//! 版本源三级回退（依次尝试）：
//!   1. jsDelivr CDN  `https://cdn.jsdelivr.net/gh/hqzzzz/DuckCM@latest/version.json`
//!   2. gh-proxy 镜像 `https://gh-proxy.com/https://raw.githubusercontent.com/hqzzzz/DuckCM/HEAD/version.json`
//!   3. GitHub raw    `https://raw.githubusercontent.com/hqzzzz/DuckCM/main/version.json`
//!
//! version.json 契约：`{"version":"2.4.0","notes":"…","file":"bin/DuckCM.exe"}`
//! 下载同样三级回退（jsDelivr → gh-proxy → github raw）。
//! 安装：exe 落地临时目录 → 生成 cmd 脚本等待本进程退出 → 覆盖 → 重启。

use std::io::Read;
use std::path::PathBuf;
use std::sync::mpsc::{Receiver, TryRecvError};

pub const REPO: &str = "hqzzzz/DuckCM";
/// 本地版本（与 Cargo.toml 同步，显示于顶栏）
pub const VERSION: &str = env!("CARGO_PKG_VERSION");

fn ver_sources(file: &str) -> Vec<String> {
    vec![
        format!("https://cdn.jsdelivr.net/gh/{REPO}@latest/{file}"),
        format!("https://gh-proxy.com/https://raw.githubusercontent.com/{REPO}/HEAD/{file}"),
        format!("https://raw.githubusercontent.com/{REPO}/main/{file}"),
    ]
}

#[derive(serde::Deserialize)]
struct RemoteMeta {
    version: String,
    #[serde(default)]
    notes: String,
    /// 仓库内 exe 相对路径
    file: String,
}

/// 后台线程 → UI 消息
enum Msg {
    Latest,
    Avail { ver: String, notes: String },
    Prog(f64),
    Done { path: PathBuf, ver: String },
    Fail(String),
}

pub enum UpdState {
    Idle,
    Checking,
    /// 已是最新
    Latest,
    /// 发现新版本
    Avail { ver: String, notes: String },
    Downloading(f64),
    /// 下载完成，等待重启应用
    Ready { path: PathBuf, ver: String },
    Failed(String),
}

pub struct Updater {
    pub state: UpdState,
    rx: Option<Receiver<Msg>>,
    ctx: Option<egui::Context>,
}

impl Updater {
    pub fn new() -> Self {
        Self {
            state: UpdState::Idle,
            rx: None,
            ctx: None,
        }
    }

    fn spawn(&mut self, ctx: &egui::Context, job: &'static str, what: u8) {
        let (tx, rx) = std::sync::mpsc::channel::<Msg>();
        self.rx = Some(rx);
        self.ctx = Some(ctx.clone());
        let _ = job;
        std::thread::spawn(move || match what {
            0 => check_worker(tx),
            1 => download_worker(tx),
            _ => {}
        });
    }

    /// 发起版本检查（非阻塞）
    pub fn check(&mut self, ctx: &egui::Context) {
        if matches!(self.state, UpdState::Checking | UpdState::Downloading(_)) {
            return;
        }
        self.state = UpdState::Checking;
        self.spawn(ctx, "check", 0);
    }

    /// 发现新版本后点击下载（非阻塞；下载路径按 version.json 的 file 字段）
    pub fn download(&mut self, ctx: &egui::Context) {
        if !matches!(self.state, UpdState::Avail { .. }) {
            return;
        }
        self.state = UpdState::Downloading(0.0);
        self.spawn(ctx, "download", 1);
    }

    /// 重启并替换 exe；成功时本函数不返回（进程退出）
    pub fn apply(&self) -> std::io::Result<()> {
        let (path, _ver) = match &self.state {
            UpdState::Ready { path, ver } => (path.clone(), ver.clone()),
            _ => return Ok(()),
        };
        let exe = std::env::current_exe()?;
        let cmd_path = std::env::temp_dir().join("duckcm_update.cmd");
        let script = format!(
            "@echo off\r\n:wait\r\ntasklist /fi \"PID eq {}\" 2>nul | findstr /i \"DuckCM\" >nul\r\n\
if not errorlevel 1 (timeout /t 1 /nobreak >nul & goto wait)\r\n\
move /y \"{}\" \"{}\" >nul\r\nstart \"\" \"{}\"\r\ndel \"%~f0\"\r\n",
            std::process::id(),
            path.display(),
            exe.display(),
            exe.display()
        );
        std::fs::write(&cmd_path, script)?;
        std::process::Command::new("cmd")
            .args(["/C", &cmd_path.to_string_lossy()])
            .spawn()?;
        std::process::exit(0);
    }

    /// 每帧收割后台消息；返回是否需要持续重绘（进行中）
    pub fn poll(&mut self) -> bool {
        let Some(rx) = &self.rx else { return false };
        loop {
            match rx.try_recv() {
                Ok(m) => {
                    self.state = match m {
                        Msg::Latest => UpdState::Latest,
                        Msg::Avail { ver, notes } => UpdState::Avail { ver, notes },
                        Msg::Prog(p) => UpdState::Downloading(p),
                        Msg::Done { path, ver } => UpdState::Ready { path, ver },
                        Msg::Fail(e) => UpdState::Failed(e),
                    };
                    if let Some(ctx) = &self.ctx {
                        ctx.request_repaint();
                    }
                }
                Err(TryRecvError::Empty) => break,
                Err(TryRecvError::Disconnected) => {
                    self.rx = None;
                    break;
                }
            }
        }
        self.rx.is_some()
    }
}

fn get_text(url: &str) -> Result<String, String> {
    ureq::get(url)
        .timeout(std::time::Duration::from_secs(10))
        .call()
        .map_err(|e| e.to_string())
        .and_then(|r| {
            let mut s = String::new();
            r.into_reader()
                .read_to_string(&mut s)
                .map_err(|e| e.to_string())?;
            // Windows 记事本/PowerShell 5.1 保存的 UTF-8 带 BOM，serde_json 不容
            Ok(s.trim_start_matches('\u{feff}').to_string())
        })
}

fn parse_ver(s: &str) -> (u32, u32, u32) {
    let mut v = [0u32; 3];
    for (i, part) in s.trim().trim_start_matches('v').split('.').take(3).enumerate() {
        v[i] = part.parse().unwrap_or(0);
    }
    (v[0], v[1], v[2])
}

/// 全源查询 version.json，取版本号最高者（对抗 jsDelivr @latest 缓存延迟）
fn fetch_best_meta() -> Option<RemoteMeta> {
    let mut best: Option<RemoteMeta> = None;
    for url in ver_sources("version.json") {
        if let Ok(body) = get_text(&url) {
            if let Ok(m) = serde_json::from_str::<RemoteMeta>(&body) {
                match &best {
                    Some(b) if parse_ver(&b.version) >= parse_ver(&m.version) => {}
                    _ => best = Some(m),
                }
            }
        }
    }
    best
}

fn check_worker(tx: std::sync::mpsc::Sender<Msg>) {
    match fetch_best_meta() {
        Some(m) => {
            if parse_ver(&m.version) > parse_ver(VERSION) {
                let _ = tx.send(Msg::Avail {
                    ver: m.version,
                    notes: m.notes,
                });
            } else {
                let _ = tx.send(Msg::Latest);
            }
        }
        None => {
            let _ = tx.send(Msg::Fail("所有更新源均不可达（网络或仓库未发布）".into()));
        }
    }
}

fn download_worker(tx: std::sync::mpsc::Sender<Msg>) {
    // 先取远端元数据（拿到版本与仓库内文件路径），再按 file 字段三级回退下载
    let Some(m) = fetch_best_meta() else {
        let _ = tx.send(Msg::Fail("无法读取远端版本信息".into()));
        return;
    };
    let dest = std::env::temp_dir().join(format!("DuckCM_v{}.exe", m.version));
    let urls = vec![
        format!("https://cdn.jsdelivr.net/gh/{REPO}@v{}/{}", m.version, m.file),
        format!(
            "https://gh-proxy.com/https://raw.githubusercontent.com/{REPO}/v{}/{}",
            m.version, m.file
        ),
        format!(
            "https://raw.githubusercontent.com/{REPO}/v{}/{}",
            m.version, m.file
        ),
    ];
    for url in urls {
        if try_download(&url, &dest, &tx, &m.version).is_ok() {
            return;
        }
    }
    let _ = tx.send(Msg::Fail("下载失败（所有 CDN 源均不可达）".into()));
}

fn try_download(
    url: &str,
    dest: &PathBuf,
    tx: &std::sync::mpsc::Sender<Msg>,
    ver: &str,
) -> Result<(), String> {
    let resp = ureq::get(url)
        .timeout(std::time::Duration::from_secs(300))
        .call()
        .map_err(|e| e.to_string())?;
    let total: f64 = resp
        .header("content-length")
        .and_then(|v| v.parse().ok())
        .unwrap_or(0.0);
    let mut reader = resp.into_reader();
    let mut file = std::fs::File::create(dest).map_err(|e| e.to_string())?;
    let mut buf = [0u8; 64 * 1024];
    let mut done: f64 = 0.0;
    loop {
        let n = std::io::Read::read(&mut reader, &mut buf).map_err(|e| e.to_string())?;
        if n == 0 {
            break;
        }
        std::io::Write::write_all(&mut file, &buf[..n]).map_err(|e| e.to_string())?;
        done += n as f64;
        if total > 0.0 {
            let _ = tx.send(Msg::Prog((done / total).min(1.0)));
        }
    }
    drop(file);
    // 至少 1 MB 才算合法 exe（jsDelivr 404 页极小）
    let sz = std::fs::metadata(dest).map(|m| m.len()).unwrap_or(0);
    if sz < 1_000_000 {
        let _ = std::fs::remove_file(dest);
        return Err("文件过小，疑似源无效".into());
    }
    let _ = tx.send(Msg::Done {
        path: dest.clone(),
        ver: ver.to_string(),
    });
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::parse_ver;

    #[test]
    fn version_ordering() {
        assert!(parse_ver("2.4.0") > parse_ver("2.3.0"));
        assert!(parse_ver("2.10.0") > parse_ver("2.9.9"));
        assert!(parse_ver("v2.4.1") > parse_ver("2.4.0"));
        assert_eq!(parse_ver("2.4.0"), parse_ver("2.4"));
        assert_eq!(parse_ver("garbage"), (0, 0, 0));
    }
}
