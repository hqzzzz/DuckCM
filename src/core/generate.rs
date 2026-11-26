//! 计算书生成：「生成」按钮输出两个 Word 可直接打开的文件
//!
//! - `…-设计计算书-YYYYMMDD.docx`：全年工况（几何/围护/供暖/通风降温/小窗/湿帘/给排水）
//! - `…-热负荷计算书-YYYYMMDD.docx`：冬季供暖专册，格式对齐《热负荷计算书-核算汇总》
//!
//! 正文在内存组装为 HTML，最终经 docx::build_docx 一步生成 Word 二进制
//! 图表为纯表格 HTML（底色比例条），不依赖图片/SVG，全版本兼容。

use crate::data::ProjectState;
use crate::CalcOutcome;

/// 计算书公共抬头信息
pub const COMPANY: &str = "青岛鑫光正钢结构股份有限公司";
pub const DESIGNER: &str = "yang.linhao@hotmail.com";

const STYLE: &str = r#"<style>
@page WordSection1 {
  size:595.3pt 841.9pt; margin:2.0cm 2.5cm 2.0cm 2.5cm;
  mso-header-margin:1.2cm; mso-footer-margin:1.2cm;
}
div.WordSection1 { page:WordSection1; }
body{font-family:'SimSun','宋体',serif;font-size:12pt;line-height:1.5}
h1{font-size:18pt;text-align:center;margin:6pt 0}
h2{font-size:14pt;margin:14pt 0 4pt 0;page-break-after:avoid}
p{margin:4pt 0;text-indent:24pt}
table{border-collapse:collapse;margin:4pt 0}
td,th{border:1px solid #444;padding:2pt 8pt;font-size:10.5pt}
th{background:#eef3f8}
thead{display:table-header-group}
.note{color:#555;font-size:10pt;text-indent:0}
.center{text-align:center;text-indent:0}
p.MsoHeader{border-bottom:solid #888 .5pt;padding:0cm 0cm 4pt 0cm;margin:0cm;text-align:left;tab-stops:right 453.5pt;font-size:9pt;color:#555;text-indent:0}
p.MsoFooter{border-top:solid #888 .5pt;padding:4pt 0cm 0cm 0cm;margin:0cm;text-align:center;font-size:9pt;color:#555;text-indent:0}
</style>"#;

/// 表格首行若为 `<tr><th>` 表头，包上 `<thead>` —— HTML 标准 + Word/WPS
/// 「标题行重复」：表格跨页自动重复表头
fn add_thead(body: &str) -> String {
    let mut out = String::with_capacity(body.len());
    let mut rest = body;
    while let Some(i) = rest.find("<table") {
        out.push_str(&rest[..i]);
        let tag_end = match rest[i..].find('>') {
            Some(k) => i + k + 1,
            None => {
                out.push_str(&rest[i..]);
                return out;
            }
        };
        out.push_str(&rest[i..tag_end]);
        rest = &rest[tag_end..];
        if rest.starts_with("<tr><th>") {
            if let Some(j) = rest.find("</tr>") {
                out.push_str("<thead>");
                out.push_str(&rest[..j + 5]);
                out.push_str("</thead>");
                rest = &rest[j + 5..];
            }
        }
    }
    out.push_str(rest);
    out
}

/// 正文后处理（幂等）：数据表居中 + thead 跨页表头
fn post_body(body: &str) -> String {
    let body = body.replace("<table>", "<table align=\"center\">");
    add_thead(&body)
}

fn page(title: &str, body: &str) -> String {
    let head = format!("<!-- {title} -->");
    // 统一排版：全部无属性数据表居中（有 width/align 的图表自带居中）；表头跨页重复
    let body = post_body(body);
    format!(
        "<html xmlns:o=\"urn:schemas-microsoft-com:office:office\" \
xmlns:w=\"urn:schemas-microsoft-com:office:word\"><head><meta charset=\"utf-8\">\
<meta name=ProgId content=Word.Document>{STYLE}</head>\
<body><div class=WordSection1>{head}{body}</div>\
</body></html>"
    )
}

/// 封面页（单独一页）：书名、工程、边界声明、计算口径、内核与依据标准
/// `top_pt`：封面首行大留白高度——中文册 110pt 庄重起页；英文镜像册内容多，
/// 用小留白避免封面溢出整页后被强制分页顶出空白页
fn cover_block(state: &ProjectState, book_kind: &str, top_pt: i32) -> String {
    let ht = &state.heat;
    let cl = &state.climate;
    let name = if state.name.trim().is_empty() {
        "未命名工程"
    } else {
        state.name.trim()
    };
    let lab = |k: &str, v: &str| format!("<p><b>{k}：</b>{v}</p>");
    format!(
        "<div style='page-break-after:always'>\
<h1 style='margin-top:{top_pt}pt'>{name}<br>{book_kind}</h1>\
<p class=center>（设备选型参考文件）</p>\
<div style='margin-top:36pt'>\
{p1}{p2}{p3}{p4}{p5}{p6}{p7}{p8}{p9}{p10}\
</div></div>",
        p1 = lab("工程名称", name),
        p2 = lab("生成日期", &today()),
        p3 = lab("生成软件", &format!("{COMPANY} · DuckCM 2")),
        p4 = lab(
            "室外计算参数",
            &format!("夏季 {} ℃ ／ 冬季 {} ℃", cl.outdoor_temp, ht.min_design_temp),
        ),
        p5 = lab("边界声明", "本计算书由 DuckCM 2 自动生成，为设备选型参考意见，供设备工程师复核。"),
        p6 = lab("计算口径", "冬季稳态附加耗热量法 Σ(A·K)·Δt；夏季湿帘绝热加湿（ε）＋显热平衡风量；通风量与设备配置含 10% 冗余。"),
        p7 = lab("计算内核", "Rust 实现，湿空气物性采用 PsychroLib 函数库（与 ASHRAE RP-1485 基准对照），空气状态算法依据 ASHRAE 2017 基础；湿帘采用 7090 型厂商饱和度／压降曲线；验签指纹为国密 SM3（GB/T 32905-2016）；计算链全程单元测试覆盖。"),
        p8 = "<p><b>依据标准：</b></p>",
        p9 = "<p>GB 50176-2016《民用建筑热工设计规范》——围护结构稳态传热、热桥与保温构造；</p>\
<p>GB 50015-2019《建筑给水排水设计标准》——给水管路经济流速（≤1.5 m/s）；</p>\
<p>ASHRAE Handbook—Fundamentals（2017 SI）——湿空气物性（ASHRAE RP-1485）、等温射流衰减、外墙综合温度（sol-air）、蒸发冷却效率、近地面高度温度梯度；</p>\
<p>农业行业标准 NY/T 系列（畜禽舍环境控制）——舍内 CO<sub>2</sub> 浓度与维持通风指标参照。</p>",
        p10 = lab("设备资料", "7090 型湿帘厂商饱和度／压降曲线（波高 7 mm、波纹角 45°）；风机电机样本曲线（0 Pa 自由空气量 / −20 Pa 额定，−20 Pa 静压工况）；热源机组低温制热能力曲线。"),
    )
}

/// UNIX 日 → 本地日期字符串（civil 算法，避免引入 chrono）
fn today() -> String {
    let secs = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs() as i64)
        .unwrap_or(0);
    let days = secs.div_euclid(86_400);
    let (y, m, d) = civil_from_days(days);
    format!("{y:04}-{m:02}-{d:02}")
}

/// 本地时间戳（UTC+8 近似）"YYYY-MM-DD HH:MM"——autoSave 快照标注用
pub fn now_stamp() -> String {
    let secs = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs() as i64)
        .unwrap_or(0);
    let l = secs + 8 * 3600;
    let (y, m, d) = civil_from_days(l.div_euclid(86_400));
    format!(
        "{y:04}-{m:02}-{d:02} {:02}:{:02}",
        (l % 3600) / 60,
        (l % 60)
    )
}

fn civil_from_days(z: i64) -> (i64, u32, u32) {
    let z = z + 719_468;
    let era = if z >= 0 { z } else { z - 146_096 } / 146_097;
    let doe = z - era * 146_097;
    let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365;
    let y = yoe + era * 400;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = doy - (153 * mp + 2) / 5 + 1;
    let m = if mp < 10 { mp + 3 } else { mp - 9 };
    (if m <= 2 { y + 1 } else { y }, m as u32, d as u32)
}

fn file_stem(path: &str) -> String {
    std::path::Path::new(path)
        .file_stem()
        .and_then(|s| s.to_str())
        .unwrap_or("工程")
        .to_string()
}

fn row3(a: &str, b: &str, c: &str) -> String {
    format!("<tr><td>{a}</td><td>{b}</td><td>{c}</td></tr>")
}

fn row4(a: &str, b: &str, c: &str, d: &str) -> String {
    format!("<tr><td>{a}</td><td>{b}</td><td>{c}</td><td>{d}</td></tr>")
}

/// 纯 Rust SM3 密码杂凑（GB/T 32905-2016），用于计算书数字指纹验签
pub fn sm3_hex(msg: &[u8]) -> String {
    fn p0(x: u32) -> u32 {
        x ^ x.rotate_left(9) ^ x.rotate_left(17)
    }
    fn p1(x: u32) -> u32 {
        x ^ x.rotate_left(15) ^ x.rotate_left(23)
    }
    let mut v: [u32; 8] = [
        0x7380166f, 0x4914b2b9, 0x172442d7, 0xda8a0600, 0xa96f30bc, 0x163138aa, 0xe38dee4d,
        0xb0fb0e4e,
    ];
    let bitlen = (msg.len() as u64).wrapping_mul(8);
    let mut data = msg.to_vec();
    data.push(0x80);
    while data.len() % 64 != 56 {
        data.push(0);
    }
    data.extend_from_slice(&bitlen.to_be_bytes());
    for chunk in data.chunks(64) {
        let mut w = [0u32; 68];
        let mut w2 = [0u32; 64];
        for i in 0..16 {
            w[i] = u32::from_be_bytes(chunk[i * 4..i * 4 + 4].try_into().unwrap());
        }
        for j in 16..68 {
            w[j] = p1(w[j - 16] ^ w[j - 9] ^ w[j - 3].rotate_left(15))
                ^ w[j - 13].rotate_left(7)
                ^ w[j - 6];
        }
        for j in 0..64 {
            w2[j] = w[j] ^ w[j + 4];
        }
        let (mut a, mut b, mut c, mut d, mut e, mut f, mut g, mut hh) =
            (v[0], v[1], v[2], v[3], v[4], v[5], v[6], v[7]);
        for j in 0..64usize {
            let tj: u32 = if j < 16 { 0x79cc4519 } else { 0x7a879d8a };
            let ss1 = a
                .rotate_left(12)
                .wrapping_add(e)
                .wrapping_add(tj.rotate_left((j % 32) as u32))
                .rotate_left(7);
            let ss2 = ss1 ^ a.rotate_left(12);
            let ff = if j < 16 {
                a ^ b ^ c
            } else {
                (a & b) | (a & c) | (b & c)
            };
            let gg = if j < 16 {
                e ^ f ^ g
            } else {
                (e & f) | ((!e) & g)
            };
            let tt1 = ff.wrapping_add(d).wrapping_add(ss2).wrapping_add(w2[j]);
            let tt2 = gg.wrapping_add(hh).wrapping_add(ss1).wrapping_add(w[j]);
            d = c;
            c = b.rotate_left(9);
            b = a;
            a = tt1;
            hh = g;
            g = f.rotate_left(19);
            f = e;
            e = p0(tt2);
        }
        v = [
            v[0] ^ a,
            v[1] ^ b,
            v[2] ^ c,
            v[3] ^ d,
            v[4] ^ e,
            v[5] ^ f,
            v[6] ^ g,
            v[7] ^ hh,
        ];
    }
    v.iter().map(|x| format!("{x:08x}")).collect()
}

/// PDF 签署正本 sidecar：对 PDF 全文件计算 SM3 并写 `{pdf}.sm3` 清单
/// （格式：`SM3(文件)  文件名  生成时间`，同软件下载站指纹清单做法）
pub fn write_pdf_sm3(pdf: &std::path::Path) -> Result<std::path::PathBuf, String> {
    let bytes = std::fs::read(pdf).map_err(|e| format!("读取 PDF 失败: {e}"))?;
    let fp = sm3_hex(&bytes);
    let side = pdf.with_extension("pdf.sm3");
    let name = pdf
        .file_name()
        .map(|n| n.to_string_lossy().into_owned())
        .unwrap_or_default();
    let body_txt = format!("{fp}  {name}  {}  DuckCM-2/SM3-GBT32905\n", today());
    std::fs::write(&side, body_txt).map_err(|e| format!("写清单失败: {e}"))?;
    Ok(side)
}

/// 验证 PDF 文件与 sidecar 清单一致：Ok(true)=文件未被改动
pub fn verify_pdf_sm3(pdf: &std::path::Path) -> Result<bool, String> {
    let side = pdf.with_extension("pdf.sm3");
    let list = std::fs::read_to_string(&side).map_err(|_| {
        format!(
            "缺少指纹清单 {}",
            side.file_name()
                .map(|n| n.to_string_lossy().into_owned())
                .unwrap_or_default()
        )
    })?;
    let recorded: String = list.split_whitespace().next().unwrap_or("").to_lowercase();
    if recorded.len() != 64 || !recorded.bytes().all(|b| b.is_ascii_hexdigit()) {
        return Err("指纹清单损坏".to_string());
    }
    let bytes = std::fs::read(pdf).map_err(|e| format!("读取 PDF 失败: {e}"))?;
    Ok(sm3_hex(&bytes) == recorded)
}

/// 验签：重算计算书 HTML 正文（验签页之前部分）的 SM3 并与内嵌指纹比对。
/// Ok(true)=一致；Ok(false)=指纹失配（正文被改动）；Err=非本系统计算书。
pub fn verify_doc_fingerprint(html: &str) -> Result<bool, String> {
    let tag = "<div class=WordSection1>";
    let mut start = html
        .find(tag)
        .ok_or("不是 DuckCM 计算书（缺 WordSection1）")?
        + tag.len();
    if html[start..].starts_with("<!--") {
        start = html[start..]
            .find("-->")
            .map(|k| start + k + 3)
            .ok_or("HTML 截断")?;
    }
    let mark = html.find("<div><h2>").ok_or("未找到验签页")?;
    if mark < start {
        return Err("验签页位置异常".to_string());
    }
    let pre = &html[start..mark];
    let key = "word-break:break-all'>";
    let pos = html[mark..].find(key).ok_or("验签页缺指纹行")? + mark + key.len();
    let emb: String = html[pos..].chars().take(64).collect();
    if emb.len() != 64 || !emb.bytes().all(|b| b.is_ascii_hexdigit()) {
        return Err("指纹行损坏".to_string());
    }
    Ok(sm3_hex(pre.as_bytes()) == emb)
}

/// 文档验签块（正文末尾追加）：全文 SM3 指纹 + Code128 条码，篡改即失配
fn sig_block(body: &str) -> String {
    let fp = sm3_hex(body.as_bytes());
    let short = &fp[..16];
    let (png_w, png_h, pixels) = code128_gray(short, 2);
    let _ = png_w;
    let img = base64(&png_gray8(png_w, png_h, &pixels));
    format!(
        "<div><h2>验签与防篡改声明</h2>\
<p class=note>本文档由 DuckCM 2 于 {} 自动生成，正文全部字符SM3加密：</p>\
<p class=center><img src=\"data:image/png;base64,{img}\" width=424 height=60 alt=\"fingerprint barcode\"></p>\
<p style='font-family:Consolas,monospace;font-size:9.5pt;text-align:center;word-break:break-all'>{fp}</p>\
<p style='text-align:center;color:#555;font-size:10pt'>条码为指纹前 16 位</p></div>",
        today(),
        fp = fp,
        img = img
    )
}

/// Code 128 官方宽度表（ISO 15417：值 0-102 数据、103/104/105 = START A/B/C、106 = STOP）
const CODE128_PATTERNS: [&str; 107] = [
    "212222", "222122", "222221", "121223", "121322", "131222", "122213", "122312", "132212",
    "221213", "221312", "231212", "112232", "122132", "122231", "113222", "123122", "123221",
    "223211", "221132", "221231", "213212", "223112", "312131", "311222", "321122", "321221",
    "312212", "322112", "322211", "212123", "212321", "232121", "111323", "131123", "131321",
    "112313", "132113", "132311", "211313", "231113", "231311", "112133", "112331", "132131",
    "113123", "113321", "133121", "313121", "211331", "231131", "213113", "213311", "213131",
    "311123", "311321", "331121", "312113", "312311", "332111", "314111", "221411", "431111",
    "111224", "111422", "121124", "121421", "141122", "141221", "112214", "112412", "122114",
    "122411", "142112", "142211", "241211", "221114", "413111", "241112", "134111", "111242",
    "121142", "121241", "114212", "124112", "124211", "411212", "421112", "421211", "212141",
    "214121", "412121", "111143", "111341", "131141", "114113", "114311", "411113", "411311",
    "113141", "114131", "311141", "411131", "211412", "211214", "211232", "2331112",
];

/// Code 128B 编码 → 模块序列（1=bar, 0=space）
fn code128b_bits(data: &str) -> Vec<u8> {
    let vals: Vec<u32> = data.bytes().map(|b| b as u32 - 32).collect();
    let mut sum: u32 = 104; // START B
    for (i, v) in vals.iter().enumerate() {
        sum += v * (i as u32 + 1);
    }
    let check = sum % 103;
    let mut out = Vec::new();
    let seq: Vec<u32> = std::iter::once(104u32)
        .chain(vals)
        .chain(std::iter::once(check))
        .chain(std::iter::once(106))
        .collect();
    for v in seq {
        let mut bar = true;
        for ch in CODE128_PATTERNS[v as usize].chars() {
            for _ in 0..(ch as u32 - '0' as u32) {
                out.push(if bar { 1 } else { 0 });
            }
            bar = !bar;
        }
    }
    out
}

/// 条码位图（8bit 灰度，返回 (宽, 高, 像素)，0=黑 255=白）
fn code128_gray(data: &str, module_px: u32) -> (u32, u32, Vec<u8>) {
    let bits = code128b_bits(data);
    let quiet = 10 * module_px;
    let w = bits.len() as u32 * module_px + 2 * quiet;
    let h = 60;
    let mut px = vec![255u8; (w * h) as usize];
    for (i, b) in bits.iter().enumerate() {
        if *b == 1 {
            let x0 = (quiet + i as u32 * module_px) as usize;
            for y in 0..h {
                for x in x0..x0 + module_px as usize {
                    px[(y * w) as usize + x] = 0;
                }
            }
        }
    }
    (w, h, px)
}

/// 最小 PNG 编码器（8-bit 灰度，zlib stored 块——合法无压缩）
fn png_gray8(w: u32, h: u32, gray: &[u8]) -> Vec<u8> {
    fn crc32(bytes: &[u8]) -> u32 {
        let mut c = 0xFFFF_FFFFu32;
        for &b in bytes {
            c ^= b as u32;
            for _ in 0..8 {
                let lsb = c & 1;
                c >>= 1;
                if lsb == 1 {
                    c ^= 0xEDB8_8320;
                }
            }
        }
        !c
    }
    fn chunk(out: &mut Vec<u8>, ty: &[u8; 4], data: &[u8]) {
        out.extend_from_slice(&(data.len() as u32).to_be_bytes());
        out.extend_from_slice(ty);
        out.extend_from_slice(data);
        let mut crcbuf = Vec::with_capacity(4 + data.len());
        crcbuf.extend_from_slice(ty);
        crcbuf.extend_from_slice(data);
        out.extend_from_slice(&crc32(&crcbuf).to_be_bytes());
    }
    let mut raw = Vec::with_capacity(h as usize * (w as usize + 1));
    for y in 0..h as usize {
        raw.push(0);
        raw.extend_from_slice(&gray[y * w as usize..(y + 1) * w as usize]);
    }
    let mut z = vec![0x78u8, 0x01];
    let mut i = 0usize;
    loop {
        let n = (raw.len() - i).min(0xFFFF);
        let last = i + n >= raw.len();
        z.push(last as u8);
        z.extend_from_slice(&(n as u16).to_le_bytes());
        z.extend_from_slice(&(!(n as u16)).to_le_bytes());
        z.extend_from_slice(&raw[i..i + n]);
        i += n;
        if last {
            break;
        }
    }
    let adler = {
        let (mut a, mut bb) = (1u32, 0u32);
        for &b in &raw {
            a = (a + b as u32) % 65521;
            bb = (bb + a) % 65521;
        }
        (bb << 16) | a
    };
    z.extend_from_slice(&adler.to_be_bytes());
    let mut out = b"\x89PNG\r\n\x1a\n".to_vec();
    let mut ihdr = Vec::new();
    ihdr.extend_from_slice(&w.to_be_bytes());
    ihdr.extend_from_slice(&h.to_be_bytes());
    ihdr.extend_from_slice(&[8, 0, 0, 0, 0]);
    chunk(&mut out, b"IHDR", &ihdr);
    chunk(&mut out, b"IDAT", &z);
    chunk(&mut out, b"IEND", b"");
    out
}

/// 标准 base64
fn base64(data: &[u8]) -> String {
    const T: &[u8; 64] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";
    let mut s = String::with_capacity(data.len() * 4 / 3 + 4);
    for c in data.chunks(3) {
        let b = [c[0], *c.get(1).unwrap_or(&0), *c.get(2).unwrap_or(&0)];
        let v = ((b[0] as u32) << 16) | ((b[1] as u32) << 8) | b[2] as u32;
        s.push(T[(v >> 18) as usize & 63] as char);
        s.push(T[(v >> 12) as usize & 63] as char);
        s.push(if c.len() > 1 {
            T[(v >> 6) as usize & 63] as char
        } else {
            '='
        });
        s.push(if c.len() > 2 {
            T[v as usize & 63] as char
        } else {
            '='
        });
    }
    s
}

// ────────────────────────── 图表（纯表格 HTML，Word/WPS 全兼容） ──────────────────────────

fn esc(s: &str) -> String {
    s.replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
}

/// 横向条形图：每行「标签｜色条｜数值」，色条 = 带底纹的空格 run
/// （Word 原生字符底纹，无嵌套表——Word/WPS/万兴渲染一致）
fn html_bars(title: &str, items: &[(&str, f64)], unit: &str) -> String {
    let max = items
        .iter()
        .map(|(_, v)| *v)
        .fold(0.0f64, f64::max)
        .max(1e-9);
    let colors = [
        "#2f6fd6", "#2e8b57", "#e67e22", "#8e44ad", "#c0392b", "#16a085",
    ];
    let mut o = String::from(
        "<table class=\"charttbl\" width=\"440\" align=\"center\" style=\"border:none\">",
    );
    for (i, (name, v)) in items.iter().enumerate() {
        // 色条 40 格制（最少 1 格）
        let n = ((v / max * 40.0).round() as i32).clamp(1, 40);
        let bar = "<span style='background:#2f6fd6'>&nbsp;</span>"
            .replace("#2f6fd6", colors[i % colors.len()]);
        let mut track = String::new();
        for _ in 0..n {
            track.push_str(&bar);
        }
        o += &format!(
            "<tr><td width=\"150\" align=\"right\" style=\"border:none;padding:2pt 6pt 2pt 0;white-space:nowrap\">{}</td>\
<td style=\"border:none;padding:2pt 0\" class=\"barcell\">{track}</td>\
<td width=\"95\" style=\"border:none;padding:2pt 0 2pt 6pt;white-space:nowrap\">{v:.1} {unit}</td></tr>",
            esc(name),
        );
    }
    o += "</table>";
    // 图名在图下（与原 C# 图纸习惯一致）；charttbl → docx 整表+图名不分页
    o += &format!("<p class=center><b>{}</b></p>", esc(title));
    o
}

// ────────────────────────── PNG 曲线图（逐像素绘制，WPS 兼容） ──────────────────────────

/// 极简 RGB 画布（曲线图光栅化，避免字体依赖：数字用 3×5 点阵）
struct Canvas {
    w: u32,
    h: u32,
    px: Vec<u8>,
}

const INK: (u8, u8, u8) = (40, 40, 40);
const BLUE: (u8, u8, u8) = (47, 111, 214);
const RED: (u8, u8, u8) = (192, 57, 43);
const GRID: (u8, u8, u8) = (222, 228, 235);
const PURPLE_PINK: (u8, u8, u8) = (236, 64, 165);

/// 3×5 点阵：0-9 与 '-' '.'
const GLYPHS: [&str; 12] = [
    "111101101101111", // 0
    "010110010010111", // 1
    "111001111100111", // 2
    "111001111001111", // 3
    "101101111001001", // 4
    "111100111001111", // 5
    "111100111101111", // 6
    "111001001010010", // 7
    "111101111101111", // 8
    "111101111001111", // 9
    "000000111000000", // -
    "000000000000010", // .
];

impl Canvas {
    fn new(w: u32, h: u32) -> Self {
        Self {
            w,
            h,
            px: vec![255; (w * h * 3) as usize],
        }
    }
    fn set(&mut self, x: i64, y: i64, c: (u8, u8, u8)) {
        if x < 0 || y < 0 || x >= self.w as i64 || y >= self.h as i64 {
            return;
        }
        let i = ((y * self.w as i64 + x) * 3) as usize;
        self.px[i] = c.0;
        self.px[i + 1] = c.1;
        self.px[i + 2] = c.2;
    }
    fn line(&mut self, x0: f64, y0: f64, x1: f64, y1: f64, c: (u8, u8, u8)) {
        let n = ((x1 - x0).abs().max((y1 - y0).abs())).ceil() as i64;
        for k in 0..=n {
            let t = k as f64 / n.max(1) as f64;
            self.set(
                (x0 + (x1 - x0) * t).round() as i64,
                (y0 + (y1 - y0) * t).round() as i64,
                c,
            );
        }
    }
    fn dot(&mut self, x: f64, y: f64, r: i64, c: (u8, u8, u8)) {
        for dy in -r..=r {
            for dx in -r..=r {
                if dx * dx + dy * dy <= r * r {
                    self.set(x as i64 + dx, y as i64 + dy, c);
                }
            }
        }
    }
    /// 数字文本（3×5 点阵 × scale），返回绘制宽度
    fn text(&mut self, x: i64, y: i64, s: &str, c: (u8, u8, u8), scale: i64) {
        let mut cx = x;
        for ch in s.chars() {
            let idx = match ch {
                '0'..='9' => ch as usize - '0' as usize,
                '-' => 10,
                '.' => 11,
                ' ' => {
                    cx += 4 * scale;
                    continue;
                }
                _ => continue,
            };
            for row in 0..5 {
                for col in 0..3 {
                    if GLYPHS[idx].as_bytes()[row * 3 + col] == b'1' {
                        for sy in 0..scale {
                            for sx in 0..scale {
                                self.set(
                                    cx + col as i64 * scale + sx,
                                    y + row as i64 * scale + sy,
                                    c,
                                );
                            }
                        }
                    }
                }
            }
            cx += 4 * scale;
        }
    }
    fn to_png_base64(&self) -> String {
        let img = image::RgbImage::from_raw(self.w, self.h, self.px.clone()).expect("canvas size");
        let mut buf = std::io::Cursor::new(Vec::new());
        img.write_to(&mut buf, image::ImageFormat::Png)
            .expect("png encode");
        b64(&buf.into_inner())
    }
}

/// 最小 base64（data-uri 内嵌用）
fn b64(bytes: &[u8]) -> String {
    const T: &[u8; 64] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";
    let mut s = String::new();
    for ch in bytes.chunks(3) {
        let b = [ch[0], *ch.get(1).unwrap_or(&0), *ch.get(2).unwrap_or(&0)];
        let n = ((b[0] as u32) << 16) | ((b[1] as u32) << 8) | b[2] as u32;
        s.push(T[(n >> 18) as usize & 63] as char);
        s.push(T[(n >> 12) as usize & 63] as char);
        s.push(if ch.len() > 1 {
            T[(n >> 6) as usize & 63] as char
        } else {
            '='
        });
        s.push(if ch.len() > 2 {
            T[n as usize & 63] as char
        } else {
            '='
        });
    }
    s
}

/// 房间温度曲线（制热拟合）PNG：左轴温度—热网平衡舍温（蓝）、右轴功率—
/// 单台制热能力（粉），红点=设计工况；数字 3×5 点阵，图例在图下文字说明
fn html_roomfit_chart(state: &ProjectState, out: &CalcOutcome, units: f64) -> String {
    let Some(m) = crate::heat::find_heater_model(&state.heat.heater_model_name) else {
        return String::new();
    };
    let h = &state.house;
    let base = crate::house::geometry(h).unwrap_or_default();
    let vent = &state.vent;
    let wk_wall = crate::house::wall_total_conductance(&base, vent);
    let wk_air = crate::house::air_total_conductance(&base, vent);
    let terminal_kw = out.heat.terminal_total_power_kw;
    let (w, hh_) = (760u32, 400u32);
    let (ml, mr, mt, mb) = (70.0f64, 70.0, 26.0, 56.0);
    let (x0, x1) = (-20.0f64, 15.0f64);
    let tmax = 50.0f64; // 左轴 ℃
    let kmax = ((m.capacity_kw(x1) / 40.0).ceil() * 40.0).max(40.0); // 右轴 kW
    let fx = |t: f64| ml + (t - x0) / (x1 - x0) * (w as f64 - ml - mr);
    let fy_t = |c: f64| hh_ as f64 - mb - c / tmax * (hh_ as f64 - mt - mb);
    let fy_k = |c: f64| hh_ as f64 - mb - c / kmax * (hh_ as f64 - mt - mb);
    let mut cv = Canvas::new(w, hh_);
    for i in 0..=5 {
        let tv = tmax * i as f64 / 5.0;
        let y = fy_t(tv);
        for xx in (ml as i64..(w as f64 - mr) as i64).step_by(6) {
            cv.set(xx, y.round() as i64, GRID);
        }
        cv.text(
            (ml - 10.0 - format!("{tv:.0}").chars().count() as f64 * 9.0) as i64,
            (y - 6.0) as i64,
            &format!("{tv:.0}"),
            INK,
            3,
        );
        let kv = kmax * i as f64 / 5.0;
        cv.text(
            (w as f64 - mr + 10.0) as i64,
            (fy_k(kv) - 6.0) as i64,
            &format!("{kv:.0}"),
            INK,
            3,
        );
    }
    cv.line(ml, mt, ml, hh_ as f64 - mb, INK);
    cv.line(ml, hh_ as f64 - mb, w as f64 - mr, hh_ as f64 - mb, INK);
    cv.line(w as f64 - mr, mt, w as f64 - mr, hh_ as f64 - mb, INK);
    for t in (-20..=15).step_by(5) {
        let x = fx(t as f64);
        cv.line(x, hh_ as f64 - mb, x, hh_ as f64 - mb + 6.0, INK);
        let lab = format!("{t}");
        cv.text(
            (x - lab.chars().count() as f64 * 6.0) as i64,
            (hh_ as f64 - mb + 12.0) as i64,
            &lab,
            INK,
            3,
        );
    }
    // 能力曲线（粉，右轴）与热网平衡舍温（蓝，左轴），1℃ 采样
    let q_src = |t: f64| m.capacity_kw(t) * units.max(1.0) * 0.92;
    let mut prev_cap: Option<(f64, f64)> = None;
    let mut prev_room: Option<(f64, f64)> = None;
    for ti in -200..=150 {
        let t = ti as f64 / 10.0;
        let cap = m.capacity_kw(t);
        let p = (t, cap);
        if let Some(q) = prev_cap {
            cv.line(fx(q.0), fy_k(q.1), fx(p.0), fy_k(p.1), PURPLE_PINK);
        }
        prev_cap = Some(p);
        let pt = crate::heat::heat_network_point(
            t,
            q_src(t),
            terminal_kw,
            h.roof_height,
            base.wk_floor,
            wk_wall,
            base.wk_roof,
            wk_air,
            state.heat.water_temp,
        );
        let rp = (t, pt.room_temp);
        if let Some(q) = prev_room {
            cv.line(fx(q.0), fy_t(q.1), fx(rp.0), fy_t(rp.1), BLUE);
        }
        prev_room = Some(rp);
    }
    // 设计工况红点（平衡舍温）
    let td = state.heat.min_design_temp.clamp(x0, x1);
    let pd = crate::heat::heat_network_point(
        td,
        q_src(td),
        terminal_kw,
        h.roof_height,
        base.wk_floor,
        wk_wall,
        base.wk_roof,
        wk_air,
        state.heat.water_temp,
    );
    cv.dot(fx(td), fy_t(pd.room_temp), 6, RED);
    cv.text(
        (fx(td) + 10.0) as i64,
        (fy_t(pd.room_temp) - 22.0) as i64,
        &format!("{:.1}", pd.room_temp),
        RED,
        3,
    );
    format!(
        "<p class=center><img src=\"data:image/png;base64,{}\" width=\"420\" height=\"221\"></p>",
        cv.to_png_base64()
    )
}

/// 机组制热能力曲线图：真折线 PNG（0.5℃ 采样），红点=设计工况
fn html_capacity(title: &str, model: &str, mark_t: f64) -> String {
    let Some(m) = crate::heat::find_heater_model(model) else {
        return String::new();
    };
    let (w, h) = (760u32, 400u32);
    let (ml, mr, mt, mb) = (78.0f64, 26.0, 26.0, 56.0);
    let (x0, x1) = (-20.0f64, 15.0f64);
    let ymax = ((m.capacity_kw(x1) / 20.0).ceil() * 20.0).max(20.0);
    let fx = |t: f64| ml + (t - x0) / (x1 - x0) * (w as f64 - ml - mr);
    let fy = |c: f64| h as f64 - mb - c / ymax * (h as f64 - mt - mb);
    let mut cv = Canvas::new(w, h);
    // 网格与坐标轴
    for i in 0..=5 {
        let yv = ymax * i as f64 / 5.0;
        let y = fy(yv);
        for xx in (ml as i64..(w as f64 - mr) as i64).step_by(6) {
            cv.set(xx, y.round() as i64, GRID);
        }
        cv.text(
            (ml - 10.0 - format!("{yv:.0}").chars().count() as f64 * 9.0) as i64,
            (y - 6.0) as i64,
            &format!("{yv:.0}"),
            INK,
            3,
        );
    }
    cv.line(ml, mt, ml, h as f64 - mb, INK);
    cv.line(ml, h as f64 - mb, w as f64 - mr, h as f64 - mb, INK);
    for t in (-20..=15).step_by(5) {
        let x = fx(t as f64);
        cv.line(x, h as f64 - mb, x, h as f64 - mb + 6.0, INK);
        let lab = format!("{t}");
        cv.text(
            (x - lab.chars().count() as f64 * 6.0) as i64,
            (h as f64 - mb + 12.0) as i64,
            &lab,
            INK,
            3,
        );
    }
    // 能力曲线（0.5℃ 折线）
    let mut prev: Option<(f64, f64)> = None;
    let mut k = 0i32;
    loop {
        let t = x0 + k as f64 * 0.5;
        if t > x1 {
            break;
        }
        let p = (t, m.capacity_kw(t));
        if let Some(q) = prev {
            cv.line(fx(q.0), fy(q.1), fx(p.0), fy(p.1), BLUE);
        }
        prev = Some(p);
        k += 1;
    }
    // 设计工况红点
    let cap_mark = m.capacity_kw(mark_t.clamp(x0, x1));
    cv.dot(fx(mark_t), fy(cap_mark), 6, RED);
    cv.text(
        (fx(mark_t) + 10.0) as i64,
        (fy(cap_mark) - 22.0) as i64,
        &format!("{cap_mark:.1} kW"),
        RED,
        3,
    );
    let cap_mark = crate::heat::find_heater_model(model)
        .map(|mm| mm.capacity_kw(mark_t))
        .unwrap_or(0.0);
    let cop_mark = crate::heat::find_heater_model(model)
        .map(|mm| mm.cop_at(mark_t))
        .unwrap_or(0.0);
    format!(
        "<p class=center><img src=\"data:image/png;base64,{}\" width=\"420\" height=\"221\"></p>\
<p class=center><b>{}</b>（{}）</p>\
<p class=note align=center><font color=\"#2f6fd6\">—</font> 单台制热能力三次拟合曲线 · \
<font color=\"#c0392b\">●</font> 设计工况 {mt:.0} ℃ · 单台 {cap:.1} kW · <b>COP = {cop:.2}</b>\
（COP = 制热量 ÷ 额定输入功率 {pw:.1} kW，指示值；低温端除霜未计入） 横轴 室外温度 ℃ / 纵轴 kW</p>",
        cv.to_png_base64(),
        esc(title),
        esc(model),
        mt = mark_t,
        cap = cap_mark,
        cop = cop_mark,
        pw = crate::heat::find_heater_model(model)
            .map(|mm| mm.power_kw)
            .unwrap_or(0.0)
    )
}

// ────────────────────────── 输出 ──────────────────────────

/// 生成两份计算书，返回两个文件路径。
/// `wall_layers` / `roof_layers` 为墙、屋面构造层描述（如「75mm 玻璃丝棉」，外→内），
/// 为空时按传热系数泛称「保温夹芯板」。
pub fn write_reports(
    state: &ProjectState,
    out: &CalcOutcome,
    units: f64,
    project_path: &str,
    wall_layers: &str,
    roof_layers: &str,
) -> Result<(String, String), String> {
    let stem = file_stem(project_path);
    let dir = std::path::Path::new(project_path)
        .parent()
        .map(|p| p.to_path_buf())
        .unwrap_or_else(|| std::path::PathBuf::from("."));
    let stamp = today().replace('-', "");
    let p_design = write_with_retry(
        &dir,
        &stem,
        "设计计算书",
        &stamp,
        design_book(state, out, units),
    )?;
    let p_heat = write_with_retry(
        &dir,
        &stem,
        "热负荷计算书",
        &stamp,
        heat_book(state, out, units, wall_layers, roof_layers),
    )?;
    Ok((p_design, p_heat))
}

/// 写入计算书：HTML 在内存组装，**一步生成 Word .docx（OOXML zip）**
/// （docx::build_docx——页眉页脚原生 part，零 COM 后处理）；
/// 目标被 Word/WPS 占用时自动加时间戳另存，返回最终路径
/// 本地时刻 HHMM（UTC+8）——文件被占用时的文件名消歧后缀
fn now_hhmm() -> u32 {
    let secs = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs() as i64)
        .unwrap_or(0);
    let l = secs + 8 * 3600;
    ((l % 86_400) / 3600 * 100 + (l % 3600) / 60) as u32
}

fn write_with_retry(
    dir: &std::path::Path,
    stem: &str,
    kind: &str,
    stamp: &str,
    html: String,
) -> Result<String, String> {
    let bytes = crate::docx::build_docx(&html, &today(), &html);
    let p = dir.join(format!("{stem}-{kind}-{stamp}.docx"));
    match std::fs::write(&p, &bytes) {
        Ok(()) => Ok(p.to_string_lossy().into_owned()),
        Err(_) => {
            let hhmm = now_hhmm();
            let p2 = dir.join(format!("{stem}-{kind}-{stamp}-{hhmm:04}.docx"));
            std::fs::write(&p2, &bytes)
                .map(|()| p2.to_string_lossy().into_owned())
                .map_err(|e| format!("{kind}写入失败（文件被占用）: {e}"))
        }
    }
}

// ────────────────────────── 英文设计报告（中文册镜像） ──────────────────────────

/// 验签块（EN）：与中文版同机制——正文 SM3 指纹 + Code128 条码
fn sig_block_en(body: &str) -> String {
    let fp = sm3_hex(body.as_bytes());
    let short = &fp[..16];
    let (png_w, png_h, pixels) = code128_gray(short, 2);
    let _ = png_w;
    let img = base64(&png_gray8(png_w, png_h, &pixels));
    format!(
        "<div><h2>Integrity &amp; Anti-tampering Statement</h2>\
<p class=note>This document was generated by DuckCM 2 on {date}. The full body text is fingerprinted \
with SM3 (GB/T 32905-2016):</p>\
<p class=center><img src=\"data:image/png;base64,{img}\" width=424 height=60 alt=\"fingerprint barcode\"></p>\
<p style='font-family:Consolas,monospace;font-size:9.5pt;text-align:center;word-break:break-all'>{fp}</p>\
<p style='text-align:center;color:#555;font-size:10pt'>The barcode encodes the first 16 digits of the fingerprint</p></div>",
        date = today(),
        fp = fp,
        img = img
    )
}

/// 英文设计报告：中文册正文逐语块翻译（章节/表/图/附录公式与数字 1:1 对应），
/// 指纹对**翻译后正文**计算（拖回验签口径一致），docx 用 EN 页眉页脚主题
pub fn write_report_en(
    state: &ProjectState,
    out: &CalcOutcome,
    units: f64,
    project_path: &str,
) -> Result<String, String> {
    let stem = file_stem(project_path);
    let dir = std::path::Path::new(project_path)
        .parent()
        .map(|p| p.to_path_buf())
        .unwrap_or_else(|| std::path::PathBuf::from("."));
    let stamp = today().replace('-', "");
    let body = crate::enlex::translate_en_with(
        &post_body(&design_book_body(state, out, units, true)),
        &[state.name.trim()],
    );
    let sig = sig_block_en(&body);
    let html = page("Design Report (EN)", &(body + &sig));
    let bytes =
        crate::docx::build_docx_themed(&html, &today(), &html, &crate::docx::DocTheme::en());
    let p = dir.join(format!("{stem}-DesignReport-EN-{stamp}.docx"));
    match std::fs::write(&p, &bytes) {
        Ok(()) => Ok(p.to_string_lossy().into_owned()),
        Err(_) => {
            let hhmm = now_hhmm();
            let p2 = dir.join(format!("{stem}-DesignReport-EN-{stamp}-{hhmm:04}.docx"));
            std::fs::write(&p2, &bytes)
                .map(|()| p2.to_string_lossy().into_owned())
                .map_err(|e| format!("英文报告写入失败（文件被占用）: {e}"))
        }
    }
}

// ────────────────────────── 设计计算书 ──────────────────────────

/// 设计计算书正文（各章 html 拼接，未做 post_body/签名/包装）——
/// 中文册与英文镜像册同源共用。`conclusion_break`：结论是否强制单起一页
/// （中文册 true；英文册 false——译文行高略大，附录末尾常溢出数行，
/// 强制分页会留下大半个空白页，顺排反而整页消化）
fn design_book_body(state: &ProjectState, out: &CalcOutcome, units: f64, en: bool) -> String {
    let h = &state.house;
    let r = &out.house;
    let hh = &out.heat;
    let c = &out.cool;
    let v = &out.vent;
    let b = &state.breed;
    let cl = &state.climate;
    let vn = &state.vent;
    let ht = &state.heat;
    let area = h.length * h.width;
    let dt = ht.target_room_temp - ht.min_design_temp;
    let name = if state.name.trim().is_empty() {
        "未命名工程".to_string()
    } else {
        state.name.trim().to_string()
    };

    let mut s = String::new();
    // 封面（单独一页：边界声明/口径/内核/依据标准）
    s += &cover_block(state, "设计计算书", if en { 30 } else { 110 });
    // 附录先生成：正文表格行用上标 [N] 跳转到公式书签
    let (apx, eqs) = appendix_formulas(state, out);

    s += "<h2 style='page-break-before:always'>一、工程概况</h2><table><tr><th>项目</th><th>数值</th><th>单位</th></tr>";
    s += &row3("工程名称", &name, "—");
    s += &row3(
        "室外计算参数",
        &format!(
            "夏季 {} ℃ ／ 冬季 {} ℃",
            cl.outdoor_temp, ht.min_design_temp
        ),
        "—",
    );
    s += &row3("养殖种类", breed_cn(b.breed_type), "—");
    s += &row3("养殖数量", &format!("{:.0}", b.breed_vol), "只");
    s += &row3("成型体重", &format!("{:.2}", b.breed_weight), "kg/只");
    s += &row3("舍长", &format!("{:.1}", h.length), "m");
    s += &row3("舍宽", &format!("{:.1}", h.width), "m");
    s += &row3("檐口高度", &format!("{:.2}", h.eave_height), "m");
    s += &row3("屋脊高度", &format!("{:.2}", h.roof_height), "m");
    s += &row3("吊顶高度", &format!("{:.2}", h.ceiling_height), "m");
    s += &row3("围护基础高度", &format!("{:.0}", h.wall_height_mm), "mm");
    s += &row3("建筑面积", &format!("{:.1}", area), "m²");
    s += &row3("房间容积", &format!("{:.1}", r.volume), "m³");
    s += "</table>";

    s += "<h2>二、围护结构与室外计算参数</h2><table><tr><th>参数</th><th>数值</th><th>单位</th></tr>";
    s += &row3(
        &format!("墙面传热系数 K{}", eqsup(&eqs, &["U_w"])),
        &format!("{:.2}", h.wall_u),
        "W/(m²·K)",
    );
    s += &row3(
        &format!("屋面传热系数 K{}", eqsup(&eqs, &["U_r"])),
        &format!("{:.2}", h.roof_u),
        "W/(m²·K)",
    );
    s += &row3(
        &format!("山墙（端墙）面积{}", eqsup(&eqs, &["山墙面积"])),
        &format!("{:.1}", r.area_sq),
        "m²",
    );
    s += &row3(
        &format!("侧墙（长墙+檐墙）面积{}", eqsup(&eqs, &["纵墙面积"])),
        &format!("{:.1}", r.area_zq),
        "m²",
    );
    s += &row3(
        "门窗洞口总面积（小窗+风机洞口当量）",
        &format!("{:.1}", r.area_wall_expanded),
        "m²",
    );
    s += &row3(
        &format!("屋面面积（坡面）{}", eqsup(&eqs, &["屋面面积 A_roof"])),
        &format!("{:.1}", r.area_roof_no_ceiling),
        "m²",
    );
    s += &row3(
        "吊顶面积（地面投影）",
        &format!("{:.1}", r.area_roof_ceiling),
        "m²",
    );
    s += &row3(
        &format!("地面面积{}", eqsup(&eqs, &["地面热损系数"])),
        &format!("{:.1}", r.area_floor),
        "m²",
    );
    s += &row3(
        &format!("房间容积{}", eqsup(&eqs, &["房间容积"])),
        &format!("{:.0}", r.volume),
        "m³",
    );
    s += &row3(
        "外墙表面材料",
        &format!("{}（α={:.2}）", cl.wall_surface, cl.absorptivity),
        "—",
    );
    s += &row3(
        "屋面表面材料",
        &format!("{}（α={:.2}）", cl.roof_surface, cl.roof_absorptivity),
        "—",
    );
    s += &row3(
        "室外计算温度（夏）",
        &format!("{:.1}", cl.outdoor_temp),
        "℃",
    );
    s += &row3(
        "太阳辐射",
        &format!("{:.1}", cl.solar_radiation),
        "kWh/(m²·d)",
    );
    s += &row3("外表面换热系数", &format!("{:.0}", cl.alpha_e), "W/(m²·K)");
    s += &row3("室外相对湿度（取值区间）", "60~93", "%");
    s += "</table>";

    s += &format!(
        "<h2>三、冬季供暖（设计工况）</h2><p>总耗热系数 Σ(A·K) = <b>{:.2} W/K</b>\
（地面 {:.2}、墙面含门窗缝隙 {:.2}、屋面 {:.2}、冷风渗透 {:.2} W/K），\
室内外温差 Δt = {tn:.1} − ({tw:.1}) = {dt:.1} K，房屋热负荷 Q = <b>{:.2} kW</b>。</p>",
        r.wk_floor + r.wk_wall_total + r.wk_roof + r.wk_air,
        r.wk_floor,
        r.wk_wall_total,
        r.wk_roof,
        r.wk_air,
        hh.house_heat_load_kw,
        tn = ht.target_room_temp,
        tw = ht.min_design_temp,
        dt = dt
    );
    s += "<table><tr><th>项目</th><th>数值</th><th>单位</th></tr>";
    s += &row3("热源形式", &ht.heater_model_name, "—");
    s += &row3(
        &format!("单台能力 @设计温度{}", eqsup(&eqs, &["热源单台能力"])),
        &format!("{:.2}", hh.heating_capacity_kw),
        "kW",
    );
    s += &row3(
        &format!("所需机组{}", eqsup(&eqs, &["所需机组 n_heat"])),
        &format!("{:.1}", hh.heater_units_required),
        "台",
    );
    s += &row3("实配机组", &format!("{:.0}", units), "台");
    s += &row3("末端形式", &ht.terminal_type_name, "—");
    s += &row3(
        "末端理论散热系数",
        &format!("{:.4}", ht.terminal_unit_conductance),
        "W/K·单位",
    );
    s += &row3("供回水平均温度", &format!("{:.0}", ht.water_temp), "℃");
    s += &row3(
        &format!("末端总能力（热导）{}", eqsup(&eqs, &["末端总热导"])),
        &format!("{:.2}", hh.terminal_total_power_kw),
        "kW/K",
    );
    s += &row3(
        &format!("所需末端数量{}", eqsup(&eqs, &["所需末端 n_term"])),
        &format!("{:.0}", hh.terminal_count_required),
        "—",
    );
    s += &row3("实配末端数量", &format!("{:.0}", ht.terminal_count), "—");
    s += &row3(
        &format!(
            "可达室温（设计工况）{}",
            eqsup(&eqs, &["t_eq", "热网运行点"])
        ),
        &format!("{:.2}", hh.reachable_temp),
        "℃",
    );
    s += "</table>";
    s += &html_capacity(
        "图 1　热源制热能力曲线",
        &ht.heater_model_name,
        ht.min_design_temp,
    );

    let nref_html = eqsup(&eqs, &["计算风机数"]);
    s += &format!(
        "<h2>四、夏季通风降温</h2><p>冷负荷构成：房屋冷负荷 {:.2} kW{qref} + 生物呼吸 ATP {:.2} kW{bref} \
+ 设备冷负荷 {:.1} kW = 总冷负荷 <b>{:.2} kW</b>{cref}。\
通风量按显热平衡求解（湿帘送风 t_sup = {:.1} ℃{tref}，舍内目标 t_i = {:.1} ℃{iref}，净吸热 s = {stf:.2} w/m³·h{sref}）：\
V = <b>{air:.0} m³/h</b>{vref}。\
{nfans}</p>",
        v.house_cool_load_kw,
        v.bio_cool_load_kw,
        vn.equipment_cool_load_kw,
        v.total_cool_load_kw,
        c.supply_temperature,
        c.room_temperature,
        stf = c.s_tf * 1000.0,
        air = c.air_volume,
        qref = eqsup(&eqs, &["房屋冷负荷 Q_bld"]),
        bref = eqsup(&eqs, &["生物呼吸 ATP"]),
        cref = eqsup(&eqs, &["总冷负荷"]),
        tref = eqsup(&eqs, &["湿帘物理模型"]),
        iref = eqsup(&eqs, &["舍内目标温度 t_i"]),
        sref = eqsup(&eqs, &["净吸热系数 s"]),
        vref = eqsup(&eqs, &["计算通风量 L_v"]),
        nfans = if v.aero_based {
            format!(
                "风机数按<b>空气动力学工作点</b>求解（{model} 厂商 P-Q 样本曲线{hood}）：\
系统静压 = 帘润湿压降反校值 = <b>{dp:.1} Pa</b>，\
工作点单台风量 Q(ΔP_sys) = {qop:.0} m³/h（0 Pa 自由空气量为 {q20:.0}，\
<b>可变风机效率 η = {eta:.2}</b>）：\
n = ⌈V ÷ Q(ΔP_sys)⌉ + 备用台数 = ⌈{air:.0} ÷ {qop:.0}⌉ + {sp:.0} = <b>{n:.0} 台</b>{nref}，\
实配 {act:.0} 台。",
                model = format!("{} {}", vn.fan_category, vn.fan_model_name),
                hood = if vn.fan_light_hood {
                    "，加遮光罩 ×0.91"
                } else {
                    ""
                },
                dp = v.dp_system_pa,
                qop = v.fan_q_operating,
                q20 = v.fan_q_operating / v.fan_eta_operating.max(1e-6),
                eta = v.fan_eta_operating,
                air = c.air_volume,
                sp = vn.fans_spare,
                n = v.fans_required,
                nref = nref_html.clone(),
                act = vn.fans_actual
            )
        } else {
            format!(
                "风机数 n = ⌈V ÷ (单台额定风量 × 风机效率)⌉ + 备用台数 \
= ⌈{air:.0} ÷ ({q:.0} × {eff:.2})⌉ + {sp:.0} = <b>{n:.0} 台</b>{nref}，实配 {act:.0} 台。",
                air = c.air_volume,
                q = vn.fan_unit_air_volume,
                eff = vn.fan_efficiency,
                sp = vn.fans_spare,
                n = v.fans_required,
                nref = nref_html.clone(),
                act = vn.fans_actual
            )
        }
    );
    s += "<table><tr><th>项目</th><th>数值</th><th>单位</th></tr>";
    s += &row3(
        "风机型号",
        &format!("{} {}", vn.fan_category, vn.fan_model_name),
        "—",
    );
    s += &row3(
        "遮光罩",
        if vn.fan_light_hood {
            "已加（风阻 9%，有效风量 ×0.91）"
        } else {
            "未加"
        },
        "—",
    );
    s += &row3(
        "单台自由空气量（0 Pa）",
        &format!(
            "{:.0}",
            if v.aero_based {
                // 厂商样本曲线口径（与 η 基准一致）
                v.fan_q_operating / v.fan_eta_operating.max(1e-6)
            } else {
                vn.fan_unit_air_volume
            }
        ),
        "m³/h",
    );
    if v.aero_based {
        s += &row3(
            "工作点单台风量（可变）",
            &format!("{:.0}", v.fan_q_operating),
            "m³/h",
        );
        s += &row3(
            "可变风机效率（工作点）",
            &format!("{:.2}", v.fan_eta_operating),
            "—",
        );
        s += &row3("系统静压（帘阻+动压）", &format!("{:.1}", v.dp_system_pa), "Pa");
    } else {
        s += &row3("风机效率", &format!("{:.2}", vn.fan_efficiency), "—");
    }
    s += &row3(
        &format!("计算风机数{}", eqsup(&eqs, &["计算风机数"])),
        &format!("{:.0}", v.fans_required),
        "台",
    );
    s += &row3(
        "实配风机数（含10%）",
        &format!("{:.0}", vn.fans_actual),
        "台",
    );
    s += &row3(
        &format!("GB 规范湿帘面积{}", eqsup(&eqs, &["GB 规范湿帘面积"])),
        &format!("{:.2}", c.pad_area_gb),
        "m²",
    );
    s += &row3(
        &format!("平衡法湿帘面积{}", eqsup(&eqs, &["平衡法湿帘面积"])),
        &format!("{:.2}", c.pad_area_balance),
        "m²",
    );
    s += &row3(
        &format!(
            "装机湿帘面积（额定抽吸·600 模块取整）{}",
            eqsup(&eqs, &["装机湿帘"])
        ),
        &format!("{:.1}", vn.pad_area_actual),
        "m²",
    );
    s += &row3("湿帘厚度", &format!("{:.0}", vn.pad_thickness_mm), "mm");
    s += &row3("侧墙湿帘高度", &format!("{:.2}", vn.pad_side_height), "m");
    s += &row3("侧墙湿帘层数", &format!("{:.0}", vn.pad_side_layers), "层");
    s += &row3("山墙湿帘高度", &format!("{:.2}", vn.pad_gable_height), "m");
    s += &row3("山墙湿帘道数", &format!("{:.0}", vn.pad_layers), "道");
    s += &row3(
        &format!(
            "蒸发需水量（湿帘总蒸发能力，60% RH 标准工况）{}",
            eqsup(&eqs, &["湿帘总蒸发能力"])
        ),
        &format!("{:.2}", c.evap_capacity_kg_h / 1000.0),
        "m³/h",
    );
    s += &row3(
        &format!("湿帘补水量{}", eqsup(&eqs, &["湿帘补水量 M_w"])),
        &format!("{:.2}", c.water_use),
        "m³/h",
    );
    s += &row3("蒸发吸热", &format!("{:.1}", c.cooling_power_kw), "kW");
    s += &row3(
        &format!("湿帘送风温度{}", eqsup(&eqs, &["湿帘物理模型"])),
        &format!("{:.1}", c.supply_temperature),
        "℃",
    );
    s += &row3(
        &format!("舍内相对湿度（指标）{}", eqsup(&eqs, &["舍内湿度指标"])),
        &format!("{:.1}", c.solved_humidity.unwrap_or(0.0)),
        "%",
    );
    s += &row3("舍内可达温度", &format!("{:.2}", c.room_temperature), "℃");
    s += &row3(
        &format!("体感温度{}", eqsup(&eqs, &["体感温度指数"])),
        &format!("{:.2}", c.apparent_temperature),
        "℃",
    );
    s += "</table>";
    s += &html_bars(
        "图 2　冷负荷构成",
        &[
            ("房屋冷负荷", v.house_cool_load_kw),
            ("生物呼吸 ATP", v.bio_cool_load_kw),
            ("设备冷负荷", vn.equipment_cool_load_kw),
            ("总冷负荷", v.total_cool_load_kw),
        ],
        "kW",
    );

    s += "<h2>五、进风小窗（湍、射流校核）</h2>";
    s += "<table><tr><th>口径</th><th>数值</th><th>单位</th></tr>";
    s += &row3(
        &format!("最低小窗数量{}", eqsup(&eqs, &["最低小窗数"])),
        &format!("{:.0}", v.windows_min_count),
        "个",
    );
    s += &row3(
        &format!("空间修正小窗{}", eqsup(&eqs, &["空间修正小窗数"])),
        &format!("{:.0}", v.windows_amended_count),
        "个",
    );
    s += &row3(
        &format!("最优小窗数{}", eqsup(&eqs, &["最优小窗数"])),
        &format!("{:.0}", v.windows_best_count),
        "个",
    );
    s += &row3(
        &format!("<b>计算小窗总数{}</b>", eqsup(&eqs, &["计算小窗总数"])),
        &format!("<b>{:.0}</b>", v.windows_total_count),
        "个",
    );
    s += &row3(
        &format!(
            "实配小窗（窗型 {}）{}",
            vn.window_model_name,
            eqsup(&eqs, &["小窗净间距"])
        ),
        &format!("{:.0}", vn.windows_actual),
        "个",
    );
    s += "</table>";
    s += &format!(
        "<p class='note center'>洞口风速 {ws:.2} m/s 保证舍宽中线轴线风速 ≥0.6 m/s。<br>\
等温射流界面 v(x) = v<sub>0</sub>·0.48/(0.076·x/d<sub>0</sub>+0.147){jref}。</p>",
        ws = vn.window_speed,
        jref = eqsup(&eqs, &["洞口当量直径"])
    );

    s += "<h2>六、给排水与采食</h2><table><tr><th>项目</th><th>数值</th><th>单位</th></tr>";
    s += &row3("最低供水量", &format!("{:.2}", b.breed_need_water), "m³/h");
    s += &row3(
        &format!("日采食量{}", eqsup(&eqs, &["日采食量"])),
        &format!("{:.0}", breed_feed_kg(b.breed_type) * b.breed_vol),
        "kg/日",
    );
    s += &row3(
        &format!("突发用水量{}", eqsup(&eqs, &["突发用水量 Q_突发"])),
        &format!(
            "{:.2}",
            breed_feed_kg(b.breed_type) * b.breed_vol / 3.0 * 2.2 / 1000.0
        ),
        "m³/h",
    );
    s += &row3(
        &format!(
            "湿帘蒸发需水量（总蒸发能力，60% RH 标准工况）{}",
            eqsup(&eqs, &["湿帘总蒸发能力"])
        ),
        &format!("{:.2}", c.evap_capacity_kg_h / 1000.0),
        "m³/h",
    );
    let sudden_w = breed_feed_kg(b.breed_type) * b.breed_vol / 3.0 * 2.2 / 1000.0;
    let (wq, wdn, wv) = crate::bio::water_supply_sizing(sudden_w, c.evap_capacity_kg_h / 1000.0);
    s += &row3(
        &format!("流量需求{}", eqsup(&eqs, &["水源选型"])),
        &format!("{:.1}", wq),
        "m³/h",
    );
    s += &row3(
        &format!(
            "推荐水管管径（GB 50015 经济流速 ≤1.5 m/s）{}",
            eqsup(&eqs, &["水源选型"])
        ),
        &format!("DN{:.0}（流速 {:.2} m/s）", wdn, wv),
        "mm",
    );
    s += "</table>";

    // ── 湿帘用水量与降温效率 ──
    s += "<h2>七、湿帘用水量与降温效率</h2>\
<p class=note>循环水平衡水温取室外湿球温度 τ；帘面空气按室外进气状态（t_o、φ<sub>0</sub>）。</p>";
    {
        use crate::psychro::calc_from_rel_hum;
        let p = 101_325.0_f64;
        let t_o = cl.outdoor_temp;
        let air = calc_from_rel_hum(t_o, (cl.humidity_pct / 100.0).clamp(0.05, 1.0), p);
        let t_w = air.t_wet_bulb;
        let t_sup = c.supply_temperature;
        let v_face = 1.6; // 额定面风速 [m/s]
        let th = 100.0_f64.max(vn.pad_thickness_mm.min(300.0)); // 帘厚 mm
        let mod_h_mm = vn.pad_side_height.max(0.3) * 1000.0; // 模块高 = 实际选型（1500/1800/2000）
        let e = crate::cool::pad_evaporation(
            t_w,
            t_o,
            (cl.humidity_pct / 100.0).clamp(0.05, 1.0),
            v_face,
            1.013,
            600.0,
            mod_h_mm,
            th,
        );
        let mod_face = 0.6 * mod_h_mm / 1000.0; // 单块迎风面积（600 宽 × 实际高） [m²]
        let n_mod = (vn.pad_area_actual / mod_face).max(0.0);
        let w_total = c.evap_capacity_kg_h; // 内核同源值（sl_ventilation 7090 传质法）
        let q_total = e.cooling_kw * n_mod; // 理论供冷 [kW]
        let eps = if (t_o - t_w).abs() > 0.01 {
            ((t_o - t_sup) / (t_o - t_w) * 100.0).clamp(0.0, 100.0)
        } else {
            100.0
        };
        let row = |k: &str, v: String, u: &str| {
            format!("<tr><td>{k}</td><td align=middle>{v}</td><td align=middle>{u}</td></tr>")
        };
        s += "<table><tr><th>项目</th><th>取值</th><th>单位</th></tr>";
        s += &row(
            "湿帘模块规格",
            format!(
                "600（宽度）× {:.0}（高度·按实际）× {:.0}（厚度）",
                mod_h_mm, th
            ),
            "mm",
        );
        s += &row(
            "模块等效展开水面面积 A_eq",
            format!("{:.2}", e.area_eq_m2),
            "m²",
        );
        s += &row(
            &format!(
                "循环水平衡水温 τ（室外湿球）{}",
                eqsup(&eqs, &["湿帘物理模型"])
            ),
            format!("{:.1}", t_w),
            "℃",
        );
        s += &row(
            "帘面空气温度 / 湿度",
            format!("{:.1} / {:.0}", t_o, cl.humidity_pct),
            "℃/%",
        );
        s += &row(
            "扩散系数 α(τ)（原表插值）",
            format!("{:.5}", crate::cool::diffusion_coefficient(t_w)),
            "kg/N·s",
        );
        s += &row(
            "蒸发系数 β=(α+0.00363×1.6)/10⁵",
            format!("{:.3e}", e.beta),
            "kg/N·s",
        );
        s += &row(
            "表面水分压力 p_s(τ)",
            format!("{:.0}", e.p_surface_pa),
            "Pa",
        );
        s += &row(
            "空气水分压力 p_a=φ<sub>0</sub>·p_s(t_o)",
            format!("{:.0}", e.p_air_pa),
            "Pa",
        );
        s += &row("单模块蒸发量", format!("{:.2}", e.evap_kg_h), "kg/h");
        s += &row("单模块蒸发耗冷", format!("{:.2}", e.cooling_kw), "kW");
        s += &row(
            &format!("湿帘模块数（实配面积 ÷ {:.2} m²/块）", mod_face),
            format!("{:.0}", n_mod),
            "块",
        );
        s += &row(
            &format!("总蒸发耗水（理论）{}", eqsup(&eqs, &["湿帘总蒸发能力"])),
            format!("{:.1}（≈{:.3} m³/h）", w_total, w_total / 1000.0),
            "kg/h",
        );
        s += &row("总理论供冷量", format!("{:.1}", q_total), "kW");
        s += &row(
            &format!(
                "降温效率 ε=(t_o−t_sup)/(t_o−τ){}",
                eqsup(&eqs, &["降温效率 ε"])
            ),
            format!("{:.1}", eps),
            "%",
        );
        s += "</table>";
        s += &format!(
            "<p>湿帘总蒸发能力 {:.2} m³/h，湿帘补水量 {:.2} m³/h（= 蒸发量 ×1.15 含排污飘失 + 润湿滴漏），\
循环水泵流量按 {:.1} m³/h 选型；\
高海拔地区按实际大气压对补水与效率取值进行修正。</p>",
            w_total / 1000.0,
            c.water_use,
            c.water_use * 1.3
        );
    }

    s += "<h2>八、计算公式与代入</h2>";
    s += "<p class=note>计算依据详见封面页（GB 50176-2016 · GB 50015-2019 · \
ASHRAE Handbook—Fundamentals 2017 / RP-1485 · NY/T 系列 · 设备厂商曲线）。</p>";
    s += &apx;

    s += &format!(
        "<h2{}>九、结论</h2><table><tr><th>配置项</th><th>取值</th><th>规格</th></tr>",
        if !en {
            " style='page-break-before:always'"
        } else {
            ""
        }
    );
    s += &row3(
        "风机数量",
        &format!("{:.0} 台", vn.fans_actual),
        &format!(
            "{} · {:.0} m³/h/台",
            vn.fan_model_name, vn.fan_unit_air_volume
        ),
    );
    s += &row3(
        "小窗数量",
        &format!("{:.0} 个", vn.windows_actual),
        &vn.window_model_name,
    );
    s += &row3(
        "湿帘面积",
        &format!("{:.1} m²", vn.pad_area_actual),
        &format!("厚 {:.0} mm", vn.pad_thickness_mm),
    );
    s += &row3(
        "供暖机组",
        &format!("{:.0} 台", units),
        &ht.heater_model_name,
    );
    s += &row3(
        "供暖末端",
        &format!("{:.0}", ht.terminal_count),
        &ht.terminal_type_name,
    );
    s += "</table>";
    let warns = if out.warnings.is_empty() {
        "无配置违规。".to_string()
    } else {
        format!(
            "存在配置违规：{}（若已确认设计温度无需处理）。",
            out.warnings
                .iter()
                .map(|w| format!("⚠ {}", w.full()))
                .collect::<Vec<_>>()
                .join("；")
        )
    };
    s += &format!("<p><b>{warns}</b></p>");
    s += "<p class=note>计算依据详见封面页（GB 50176-2016 · GB 50015-2019 · \
ASHRAE Handbook—Fundamentals 2017 / RP-1485 · NY/T 系列 · 设备厂商曲线）。</p>";
    s
}

/// 中文册正文（结论强制单页起 + 封面大留白）
fn design_book_body_cn(state: &ProjectState, out: &CalcOutcome, units: f64) -> String {
    design_book_body(state, out, units, false)
}

/// 设计计算书：正文 → post_body → 指纹对**最终渲染正文**计算 → 页包装
fn design_book(state: &ProjectState, out: &CalcOutcome, units: f64) -> String {
    let body = post_body(&design_book_body_cn(state, out, units));
    let sig = sig_block(&body);
    let s = body + &sig;
    page("设计计算书", &s)
}

// ────────────────────────── 热负荷计算书 ──────────────────────────

fn heat_book(
    state: &ProjectState,
    out: &CalcOutcome,
    units: f64,
    wall_layers: &str,
    roof_layers: &str,
) -> String {
    let h = &state.house;
    let r = &out.house;
    let hh = &out.heat;
    let ht = &state.heat;
    let area = h.length * h.width;
    let dt = ht.target_room_temp - ht.min_design_temp;
    let sum_ak = r.wk_floor + r.wk_wall_total + r.wk_roof + r.wk_air;
    let wall_desc = if wall_layers.is_empty() {
        "保温夹芯板（构造未提供）".to_string()
    } else {
        wall_layers.to_string()
    };
    let roof_desc = if roof_layers.is_empty() {
        "保温夹芯板（构造未提供）".to_string()
    } else {
        roof_layers.to_string()
    };

    let mut s = String::new();
    // 热负荷册附录：仅保留采暖链用到的公式（重新连续编号）
    const HEAT_KEYS: &[&str] = &[
        "屋面坡长",
        "纵墙面积",
        "山墙面积",
        "屋面面积 A_roof",
        "勒脚带面积",
        "房间容积",
        "地面热损系数",
        "墙面热损系数",
        "屋面热损系数",
        "冷风渗透热损系数",
        "墙面含开口修正",
        "冷渗含开口修正",
        "房屋热负荷 Q_h",
        "热源单台能力 P_cap",
        "所需机组 n_heat",
        "热泵性能指示 COP",
        "末端单体散热能力",
        "所需末端 n_term",
        "末端总热导 G_term",
        "末端散热能力 =",
    ];
    let (apx, eqs) = appendix_formulas_in(state, out, Some(HEAT_KEYS));
    // 封面（单独一页：边界声明/口径/内核/依据标准）
    s += &cover_block(state, "热负荷计算书", 110);
    s += &format!(
        "<p><b>本版口径：</b>分项热损采用参数单口径（稳态法 Σ(A·K)·Δt，含门窗缝隙与冷风渗透附加），\
墙面保温为 {wall_desc}，屋面保温为 {roof_desc}，室内计算温度按养殖需求 {t:.0} ℃，室外设计温度 {tw:.0} ℃。</p>",
        t = ht.target_room_temp,
        tw = ht.min_design_temp
    );

    s += "<h2>一、工程概况</h2>";
    s += &format!(
        "<p>项目：{name} 养殖舍采暖热负荷核算<br>\
舍长 {l:.1} m，舍宽 {b:.1} m，檐口高度 {e:.1} m，屋顶最高处 {rd:.2} m；\
墙面保温 {wall_desc}（K = {wu:.2} W/(m²·K)），屋面保温 {roof_desc}（K = {ru:.2} W/(m²·K)）；\
建筑面积 {a:.1} m²，房间容积 {v:.1} m³。</p>",
        name = state.name,
        l = h.length,
        b = h.width,
        e = h.eave_height,
        rd = h.roof_height,
        wu = h.wall_u,
        ru = h.roof_u,
        a = area,
        v = r.volume
    );

    // 结论（写最后：附录之后「七、结论」，与设计计算书九、结论同位——
    // 先看计算与校核，后看结论）
    let conclusion = format!(
        "<p>围护结构与冷渗总单位热损 Σ(A·K) = <b>{sum:.2} W/K</b>\
（每摄氏度温差约 {sum1:.1} kW）。\
设计热负荷按 Δt = {dt:.1} K 取值：<b>Q ≈ {q:.1} kW</b>{qref}。\
热源采用 {model} 空气源机组（设计温度下单台 {cap:.2} kW{cref}），\
所需 {need:.1} 台{nref}，实配 <b>{uni:.0} 台</b>\
（含 10% 冗余；入舍可供 = 台数×单台×拟合×热网系数 0.92 ≈ <b>{tot:.1} kW ≥ Q</b>）。\
末端采用 {term}，实配 {tc:.0}，末端总能力 {tp:.2} kW/K（Δt={dtw:.0} K 时输出 {out:.1} kW{tref}）。</p>",
        sum = sum_ak,
        sum1 = sum_ak / 1000.0,
        dt = dt,
        q = hh.house_heat_load_kw,
        qref = eqsup(&eqs, &["房屋热负荷 Q_h"]),
        model = ht.heater_model_name,
        cap = hh.heating_capacity_kw,
        cref = eqsup(&eqs, &["热泵性能指示 COP"]),
        need = hh.heater_units_required,
        nref = eqsup(&eqs, &["所需机组 n_heat"]),
        uni = units,
        // 入舍可供 = units × (P×fit×0.92)，其中 P×fit×0.92 = Q ÷ 需求台数
        tot = units * hh.house_heat_load_kw / hh.heater_units_required.max(1e-6),
        term = ht.terminal_type_name,
        tc = ht.terminal_count,
        tp = hh.terminal_total_power_kw,
        dtw = (ht.water_temp - ht.target_room_temp).max(0.0),
        out = hh.terminal_total_power_kw * (ht.water_temp - ht.target_room_temp).max(0.0),
        tref = eqsup(&eqs, &["末端散热能力"])
    );

    s += "<h2>二、前提</h2><p><b>2.1 采用参数</b>（参数单口径）</p>";
    s += "<table><tr><th>参数</th><th>数值</th><th>单位</th><th>备注</th></tr>";
    s += &row4(
        &format!("屋面面积{}", eqsup(&eqs, &["屋面坡长"])),
        &format!("{:.2}", r.area_roof_no_ceiling),
        "m²",
        "坡面展开；有吊顶时热工分界面按吊顶面积（长×宽）计",
    );
    s += &row4(
        &format!("屋面热损{}", eqsup(&eqs, &["屋面热损系数"])),
        &format!("{:.2}", r.wk_roof),
        "W/K",
        &format!("折算 {:.2} W/(m²·K)", h.roof_u),
    );
    s += &row4("地面面积", &format!("{:.2}", area), "m²", "舍长 × 舍宽");
    s += &row4(
        &format!("地面热损{}", eqsup(&eqs, &["地面热损系数"])),
        &format!("{:.2}", r.wk_floor),
        "W/K",
        &format!("温差分区法，折算 {:.2} W/(m²·K)", r.wk_floor / area),
    );
    s += &row4(
        &format!(
            "墙面面积（山墙＋纵墙）{}",
            eqsup(&eqs, &["山墙面积", "纵墙面积"])
        ),
        &format!("{:.2}", r.area_wall_expanded),
        "m²",
        "含门窗洞口",
    );
    s += &row4(
        &format!("墙面热损{}", eqsup(&eqs, &["墙面热损系数"])),
        &format!("{:.2}", r.wk_wall_total),
        "W/K",
        &format!("含门窗与设备渗透附加，基准 {:.2} W/(m²·K)", h.wall_u),
    );
    s += &row4(
        &format!("房间容积{}", eqsup(&eqs, &["房间容积"])),
        &format!("{:.2}", r.volume),
        "m³",
        "净容积",
    );
    s += &row4(
        &format!("冷风渗透热损{}", eqsup(&eqs, &["冷风渗透热损系数"])),
        &format!("{:.2}", r.wk_air),
        "W/K",
        &format!(
            "反推换气次数约 {:.2} 次/h",
            r.wk_air / (0.34 * r.volume).max(1.0)
        ),
    );
    s += "</table>";
    s += &html_bars(
        "图 1　围护结构分项热损 Σ(A·K)",
        &[
            ("墙面（含缝隙）", r.wk_wall_total),
            ("屋面", r.wk_roof),
            ("冷风渗透", r.wk_air),
            ("地面", r.wk_floor),
        ],
        "W/K",
    );

    s += "<p><b>2.2 围护结构保温构造</b></p>";
    s += "<table><tr><th>部位</th><th>保温构造（外→内）</th><th>传热系数 K</th></tr>";
    s += &format!(
        "<tr><td>墙面</td><td>{wall_desc}　+　内外彩色涂层钢板面（α={:.2}：{}）</td><td>{:.2} W/(m²·K)</td></tr>",
        state.climate.absorptivity, state.climate.wall_surface, h.wall_u
    );
    s += &format!(
        "<tr><td>屋面</td><td>{roof_desc}　+　防水面层</td><td>{:.2} W/(m²·K)</td></tr>",
        h.roof_u
    );
    s += &format!(
        "<tr><td>地面</td><td>土壤蓄热温差分区法（周边 2 m 带加强）</td><td>折算 {:.2} W/(m²·K)</td></tr>",
        r.wk_floor / area
    );
    s += "</table>";

    s += "<p><b>2.3 设计条件</b></p>";
    s += "<table><tr><th>项目</th><th>取值</th><th>说明</th></tr>";
    s += &row3(
        "室外设计温度 t_w",
        &format!("{:.1} ℃", ht.min_design_temp),
        "供暖参数单",
    );
    s += &row3(
        "室内计算温度 t_n",
        &format!("{:.1} ℃", ht.target_room_temp),
        "养殖需求舍内温度",
    );
    s += &row3("计算温差 Δt", &format!("{:.1} K", dt), "t_n − t_w");
    s += &row3(
        "墙面材料吸收系数",
        &format!("α = {:.2}", state.climate.absorptivity),
        &state.climate.wall_surface,
    );
    s += &row3(
        "屋面材料吸收系数",
        &format!("α = {:.2}", state.climate.roof_absorptivity),
        &state.climate.roof_surface,
    );
    s += "</table>";

    // 三、设计取值（独立章，紧排在热源校核之前）
    s += &format!(
        "<h2>三、设计取值</h2><p>设备容量按设计工况热负荷 Q = {:.1} kW 配置（最劣条件 Δt = {:.1} K）；\
运行期随室外温度回升，负荷与机组电功率同步下降（COP 升高），按下方校核表分档启停机组节能：\
负荷率低于约 40% 的温区可减开一台（余量仍 ≥ 需求），回温期与夜间低温段全开。\
热网可用供热按单台能力 × 实配台数 × 热网系数 0.92 计。</p>",
        hh.house_heat_load_kw, dt
    );
    // 每 5℃ 热网校核表：平衡舍温 / 单台能力·COP·电功率 / 实配总供热 / 负荷率
    s += "<table><tr><th>室外温度 ℃</th><th>热网平衡舍温 ℃</th><th>单台能力 kW</th>\
<th>单台 COP</th><th>单台电功率 kW</th><th>实配总供热 kW</th><th>负荷率 %</th></tr>";
    let w_sum_h = r.wk_floor + r.wk_wall_total + r.wk_roof + r.wk_air;
    let q_design = (ht.target_room_temp - ht.min_design_temp) * w_sum_h / 1000.0;
    let mut temps5: Vec<f64> = (-20i32..=15).step_by(5).map(|t| t as f64).collect();
    if temps5
        .iter()
        .all(|x| (*x - ht.min_design_temp).abs() > 0.01)
    {
        temps5.push(ht.min_design_temp);
        temps5.sort_by(|a, b| a.total_cmp(b));
    }
    for t in temps5 {
        let (cap, cop) = crate::heat::find_heater_model(&ht.heater_model_name)
            .map(|mm| (mm.capacity_kw(t), mm.cop_at(t)))
            .unwrap_or((0.0, 0.0));
        let pow = if cop > 0.0 { cap / cop } else { 0.0 };
        let q_src_t = cap * units.max(1.0) * 0.92;
        let pt = crate::heat::heat_network_point(
            t,
            q_src_t,
            hh.terminal_total_power_kw,
            h.roof_height,
            r.wk_floor,
            r.wk_wall_total,
            r.wk_roof,
            r.wk_air,
            ht.water_temp,
        );
        let load_pct = if q_design > 0.0 {
            ((ht.target_room_temp - t) * w_sum_h / 1000.0 / q_design) * 100.0
        } else {
            0.0
        };
        s += &format!(
            "<tr><td>{t:.0}</td><td>{:.1}</td><td>{cap:.1}</td><td>{cop:.2}</td>\
<td>{pow:.1}</td><td>{:.1}</td><td>{load_pct:.0}</td></tr>",
            pt.room_temp, q_src_t
        );
    }
    s += "</table>";
    s += "<p class=note>负荷率 =（需求温度 − 外温）× Σ(A·K) ÷ 设计点需求（电功率 = 能力 ÷ COP）。\
<b>蓝线 = 热网平衡舍温（左轴 ℃）、粉线 = 单台制热能力（右轴 kW）、\
<font color=\"#c0392b\">●</font> 设计工况平衡点。</b></p>";
    s += &html_roomfit_chart(state, out, units);
    s += "<p class=center><b>图 2　房间温度曲线 · 制热拟合</b></p>";
    s += "<p class=note>说明：曲线「舍温」为热网能量平衡运行工况值——机组不足时台数↑则舍温、运行供水温度、\
末端散热同升；机组充足后饱和于末端-围护平衡点，该上限即「可达温度(供水50°C)」。\
继续增机不再增益，此时提高舍温的手段为增加末端数量、提高供水温度或降低围护热损。</p>";

    s += "<h2>四、热源校核</h2>";
    s += &format!(
        "<p class=note>能力校核按热负荷 Q = {q:.1} kW{qref}（设计工况 Δt = {dt:.1} K）恒定，\
机组能力按 {model} 低温工况曲线折算（含空气能拟合系数）；所需台数 {nref}，单台 COP {cref}。</p>",
        q = hh.house_heat_load_kw,
        qref = eqsup(&eqs, &["房屋热负荷 Q_h"]),
        dt = dt,
        model = ht.heater_model_name,
        nref = eqsup(&eqs, &["所需机组 n_heat"]),
        cref = eqsup(&eqs, &["热泵性能指示 COP"])
    );
    s += "<table><tr><th>室外温度 ℃</th><th>单台能力 kW</th><th>单台 COP</th><th>所需台数</th><th>实配台数</th><th>实配总能力 kW</th><th>校核</th></tr>";
    let mut temps: Vec<f64> = vec![ht.min_design_temp];
    for t in [-15.0, -10.0, -5.0, 0.0] {
        if temps.iter().all(|x| (*x - t).abs() > 0.01) {
            temps.push(t);
        }
    }
    temps.sort_by(|a, b| a.total_cmp(b));
    for t in temps {
        let cap = crate::heat::find_heater_model(&ht.heater_model_name)
            .map(|m| m.capacity_kw(t))
            .unwrap_or(0.0);
        let cop = crate::heat::find_heater_model(&ht.heater_model_name)
            .map(|m| m.cop_at(t))
            .unwrap_or(0.0);
        let need = if cap > 0.0 {
            (hh.house_heat_load_kw / cap).ceil().max(1.0)
        } else {
            0.0
        };
        let tot = units * cap;
        let ok = if tot + 0.05 >= hh.house_heat_load_kw {
            "满足"
        } else {
            "不足"
        };
        s += &format!(
            "<tr><td>{t:.0}</td><td>{cap:.1}</td><td>{cop:.2}</td><td>{need:.0}</td><td>{uni:.0}</td><td>{tot:.1}</td><td>{ok}</td></tr>",
            uni = units
        );
    }
    s += "</table>";
    s += &html_capacity(
        "图 3　热源制热能力曲线",
        &ht.heater_model_name,
        ht.min_design_temp,
    );

    s += "<h2>五、末端校核</h2>";
    let dtw = (ht.water_temp - ht.target_room_temp).max(0.0);
    s += "<table><tr><th>项目</th><th>数值</th><th>单位</th><th>说明</th></tr>";
    s += &row4("末端形式", &ht.terminal_type_name, "—", "末端库（RG.json）");
    s += &row4(
        "末端理论散热系数",
        &format!("{:.4}", ht.terminal_unit_conductance),
        "W/K",
        "单体每温差散热量",
    );
    s += &row4(
        "供回水平均温度",
        &format!("{:.0}", ht.water_temp),
        "℃",
        "供暖参数单",
    );
    s += &row4(
        "室内计算温度",
        &format!("{:.0}", ht.target_room_temp),
        "℃",
        "养殖需求舍内温度",
    );
    s += &row4("末端计算温差", &format!("{:.1}", dtw), "K", "供水 − 室内");
    s += &row4(
        &format!("末端总能力（热导）{}", eqsup(&eqs, &["末端散热能力"])),
        &format!("{:.2}", hh.terminal_total_power_kw),
        "kW/K",
        "实配数量 × 单体散热系数 ÷ 1000",
    );
    s += &row4(
        &format!("末端散热能力（Δt 下）{}", eqsup(&eqs, &["末端散热能力"])),
        &format!(
            "{:.1}",
            hh.terminal_total_power_kw * dtw * ht.terminal_efficiency
        ),
        "kW",
        &format!("总能力 × Δt × 效率 {:.2}", ht.terminal_efficiency),
    );
    s += &row4(
        &format!("所需末端数量{}", eqsup(&eqs, &["所需末端 n_term"])),
        &format!("{:.0}", hh.terminal_count_required),
        "—",
        if hh.terminal_total_power_kw * dtw * ht.terminal_efficiency + 0.5 >= hh.house_heat_load_kw
        {
            "散热 ≥ 热负荷，满足"
        } else {
            "散热 < 热负荷，需增加末端"
        },
    );
    s += &row4(
        "实配末端数量",
        &format!("{:.0}", ht.terminal_count),
        "—",
        "实际应用",
    );
    s += "</table>";
    s += &html_bars(
        "图 4　供热平衡校核（设计工况）",
        &[
            ("房屋热负荷 Q", hh.house_heat_load_kw),
            (
                "机组实配能力",
                units
                    * crate::heat::find_heater_model(&ht.heater_model_name)
                        .map(|m| m.capacity_kw(ht.min_design_temp))
                        .unwrap_or(0.0),
            ),
            (
                "末端散热能力",
                hh.terminal_total_power_kw * dtw * ht.terminal_efficiency,
            ),
        ],
        "kW",
    );

    s += "<h2>六、计算公式与代入</h2>";
    s += "<p class=note>计算依据详见封面页（GB 50176-2016 · GB 50015-2019 · \
ASHRAE Handbook—Fundamentals 2017 / RP-1485 · NY/T 系列 · 设备厂商曲线）。</p>";
    s += &apx;
    // 结论写最后（附录之后、单起一页；验签块随后同页顺排）
    s += "<h2 style='page-break-before:always'>七、结论</h2>";
    s += &conclusion;
    let warns = if out.warnings.is_empty() {
        "无配置违规。".to_string()
    } else {
        format!(
            "存在配置违规：{}（若已确认设计温度无需处理）。",
            out.warnings
                .iter()
                .map(|w| format!("⚠ {}", w.full()))
                .collect::<Vec<_>>()
                .join("；")
        )
    };
    s += &format!("<p><b>{warns}</b></p>");
    let body = post_body(&s);
    let sig = sig_block(&body);
    let s = body + &sig;
    page("热负荷计算书", &s)
}

/// 将 .docx（一步生成，页眉页脚原生 part）导出为 PDF（签署正本）：
/// 固定格式导出即可，无任何注入；优先 WPS（KWPS.Application），失败回退 MS Word。
pub fn export_pdf(doc: &std::path::Path) -> Result<std::path::PathBuf, String> {
    let pdf = doc.with_extension("pdf");
    let d = doc.to_string_lossy().replace('\'', "''");
    let p = pdf.to_string_lossy().replace('\'', "''");
    let ps = format!(
        "$w=$null;try{{$w=New-Object -ComObject KWPS.Application}}catch{{$w=New-Object -ComObject Word.Application}};\
if($null -eq $w){{throw 'NO_COM_HOST'}};$w.Visible=$false;try{{$d=$w.Documents.Open('{d}');\
$d.ExportAsFixedFormat('{p}',17);$d.Close($false)}}finally{{$w.Quit()}}"
    );
    run_com(&ps).and_then(|()| {
        if pdf.exists() {
            Ok(pdf)
        } else {
            Err("COM 未产出 PDF".to_string())
        }
    })
}

/// 删除文件，失败时短暂重试——Word/WPS COM 的 Quit 之后文件锁常滞留数百毫秒，
/// 立即删除会失败导致中间 .docx 与最终 .pdf 并存
pub fn delete_with_retry(path: &std::path::Path) {
    for i in 0..10 {
        if std::fs::remove_file(path).is_ok() {
            return;
        }
        std::thread::sleep(std::time::Duration::from_millis(300 + i * 200));
    }
    let _ = std::fs::remove_file(path);
}

/// 隐藏窗口执行 PowerShell COM 脚本
fn run_com(ps: &str) -> Result<(), String> {
    use std::os::windows::process::CommandExt;
    let out = std::process::Command::new("powershell")
        .args(["-NoProfile", "-ExecutionPolicy", "Bypass", "-Command", ps])
        .creation_flags(0x0800_0000) // CREATE_NO_WINDOW
        .output()
        .map_err(|e| format!("powershell 启动失败: {e}"))?;
    if out.status.success() {
        Ok(())
    } else {
        let err = String::from_utf8_lossy(&out.stderr);
        Err(err.trim().chars().take(200).collect::<String>())
    }
}

/// 附录：公式表 + 公式索引（正文表格行以 `<sup>` 链接跳查到 `#eqN` 书签）
///
/// 返回 `(附录 html, [(公式文本, 编号)])`
fn appendix_formulas(state: &ProjectState, out: &CalcOutcome) -> (String, Vec<(String, usize)>) {
    appendix_formulas_in(state, out, None)
}

/// 带筛选的附录生成：`keep = Some(keys)` 时仅保留公式文本命中任一关键词的
/// 条目（重新连续编号，索引同步），符号表换用热负荷册精简版——
/// 热负荷计算书不携带湿帘/通风/给排水等未用公式
fn appendix_formulas_in(
    state: &ProjectState,
    out: &CalcOutcome,
    keep: Option<&[&str]>,
) -> (String, Vec<(String, usize)>) {
    let h = &state.house;
    let r = &out.house;
    let hh = &out.heat;
    let c = &out.cool;
    let v = &out.vent;
    let b = &state.breed;
    let cl = &state.climate;
    let vn = &state.vent;
    let ht = &state.heat;

    let rise = h.roof_height - h.eave_height;
    let slope = (rise * rise + h.width * h.width / 4.0).sqrt();
    let dt = ht.target_room_temp - ht.min_design_temp;
    let t_wall = crate::heat::wall_surface_temp(
        cl.outdoor_temp,
        cl.solar_radiation,
        cl.absorptivity,
        cl.alpha_e,
    );
    let wall_open =
        r.wk_wall_base + vn.fans_actual * 9.8 + vn.windows_actual * vn.window_area * 5.0;
    let aperture =
        vn.fans_actual * 1.4 * 4.0 * 4.2 + vn.windows_actual * vn.window_area.sqrt() * 4.0;
    let air_open = r.wk_infiltration + aperture * 0.24 / 0.86 * 1.2;
    let d0 = (4.0 * vn.window_area / std::f64::consts::PI).sqrt();
    let k_jet = 0.48 / (0.076 * (h.width / 2.0) / d0 + 0.147);
    let fan_ep = vn.fan_unit_air_volume * vn.fan_efficiency;
    let dtw = (ht.water_temp - ht.target_room_temp).max(0.0);
    let unit_power = dtw * ht.terminal_unit_conductance / 1000.0;

    let mut items: Vec<(String, String, String)> = Vec::new();
    let mut n = 0usize;
    let mut add = |num: &str, f: String, res: String| {
        if let Some(ks) = keep {
            if !ks.iter().any(|k| f.contains(k)) {
                return;
            }
        }
        n += 1;
        items.push((format!("（{n}）{num}"), f, res));
    };

    // ── 几何与围护 ──
    add(
        "",
        "屋面坡长 Lₛ = √((屋脊高−檐口高)² + (舍宽/2)²)".to_string(),
        format!(
            "√({:.2}² + {:.2}²) = √({:.4}+{:.2}) = √{:.4} = {:.2} m",
            rise,
            h.width / 2.0,
            rise * rise,
            h.width * h.width / 4.0,
            rise * rise + h.width * h.width / 4.0,
            slope
        ),
    );
    add(
        "",
        "纵墙面积 A_long = 2·舍长·檐口高".to_string(),
        format!(
            "2 × {:.1} × {:.1} = {:.1} m²",
            h.length, h.eave_height, r.area_zq
        ),
    );
    add(
        "",
        "山墙面积 A_gable = 2·(舍宽·檐口 + 舍宽·坡高/2)".to_string(),
        format!(
            "2 × ({:.1}×{:.1} + {:.1}×{:.2}/2) = 2 × ({:.2}+{:.2}) = {:.2} m²",
            h.width,
            h.eave_height,
            h.width,
            rise,
            h.width * h.eave_height,
            h.width * rise / 2.0,
            r.area_sq
        ),
    );
    add(
        "",
        "屋面面积 A_roof = 2·舍长·Lₛ（坡面）；吊顶面积 = 舍长×舍宽（水平平顶，热工分界面）"
            .to_string(),
        format!(
            "2 × {:.1} × {:.2} = {:.2} m²；吊顶 {:.1} × {:.1} = {:.1} m²",
            h.length, slope, r.area_roof_no_ceiling, h.length, h.width, r.area_roof_ceiling
        ),
    );
    add(
        "",
        "勒脚带面积 A_base = 2·舍长·围护高(mm)/1000".to_string(),
        format!(
            "2 × {:.1} × {:.0} / 1000 = {:.1} m²",
            h.length, h.wall_height_mm, r.area_wall_expanded
        ),
    );
    add(
        "",
        "房间容积 V = 舍长·舍宽·檐口 + 舍宽·坡高/2·舍长".to_string(),
        {
            // 与 house::geometry 同分支逐步展开
            if h.has_ceiling && h.ceiling_height <= h.eave_height {
                format!(
                    "有吊顶（吊顶 {:.2} ≤ 檐口 {:.2}）：V = {:.1}×{:.1}×{:.2} = {:.2} m³",
                    h.ceiling_height, h.eave_height, h.length, h.width, h.ceiling_height, r.volume
                )
            } else if h.has_ceiling {
                let over = h.ceiling_height - h.eave_height;
                let shrink = (1.0 - over / rise.max(1e-6)).clamp(0.0, 1.0);
                format!(
                    "有吊顶（吊顶高出檐口 {:.2} m）：V = {:.1}×{:.1}×{:.2} + {:.1}×{:.1}×(1+{:.2})/2×{:.2} = {:.2} m³",
                    over,
                    h.length,
                    h.width,
                    h.eave_height,
                    h.length,
                    h.width,
                    shrink,
                    over,
                    r.volume
                )
            } else {
                format!(
                    "无吊顶：下部 {:.1}×{:.1}×{:.1} = {:.2}；屋面段 {:.1}×{:.2}/2×{:.1} = {:.2}；V = {:.2} m³",
                    h.length,
                    h.width,
                    h.eave_height,
                    h.length * h.width * h.eave_height,
                    h.width,
                    rise,
                    h.length,
                    h.width * rise / 2.0 * h.length,
                    r.volume
                )
            }
        },
    );
    add(
        "",
        "地面热损系数 K_floor = 0.5246·A_floor（A_floor = 舍长×舍宽）（四层分区 0.87×0.30+0.47×0.26+0.35×0.23+0.29×0.21）".to_string(),
        format!(
            "A_floor = {:.1}×{:.1} = {:.1} m²；0.5246 × {:.1} = {:.2} W/K",
            h.length, h.width, r.area_floor, r.area_floor, r.wk_floor
        ),
    );
    add(
        "",
        "墙面热损系数 K_wall = (A_gable+A_long)·U_w + A_base×2.5（缝隙附加已并入显式开口项）"
            .to_string(),
        format!(
            "({:.2}+{:.1}) = {:.2} m² × {:.4} = {:.2}；勒脚 {:.1}×2.5 = {:.2}；Σ = {:.2} W/K",
            r.area_sq,
            r.area_zq,
            r.area_sq + r.area_zq,
            h.wall_u,
            (r.area_sq + r.area_zq) * h.wall_u,
            r.area_wall_expanded,
            r.area_wall_expanded * 2.5,
            r.wk_wall_base
        ),
    );
    add(
        "",
        "屋面热损系数 K_roof = A_roof·U_r × 1.06（热桥+无吊顶合并附加）".to_string(),
        format!(
            "A_roof = {:.1} m²（{}）；{:.1} × {:.4} × 1.06 = {:.2} W/K",
            r.area_roof,
            if h.has_ceiling { "吊顶水平投影" } else { "坡面展开" },
            r.area_roof,
            h.roof_u,
            r.wk_roof
        ),
    );
    add(
        "",
        "冷风渗透热损系数 K_inf = 0.5·V·ρ_air·c_p/0.86 = 0.5·V·1.293·0.24/0.86".to_string(),
        format!(
            "0.5 × {:.2} = {:.2}；×1.293 = {:.2}；×0.24 ÷ 0.86 = {:.2} W/K",
            r.volume,
            0.5 * r.volume,
            0.5 * r.volume * 1.293,
            r.wk_infiltration
        ),
    );
    add(
        "",
        "墙面含开口修正 K_wall′ = K_wall + n_fan×9.8 + n_win·A_win×5".to_string(),
        format!(
            "{:.2} + {:.0}×9.8 = {:.1} + {:.0}×{:.4}×5 = {:.2}；Σ = {:.2} W/K",
            r.wk_wall_base,
            vn.fans_actual,
            vn.fans_actual * 9.8,
            vn.windows_actual,
            vn.window_area,
            vn.windows_actual * vn.window_area * 5.0,
            wall_open
        ),
    );
    add(
        "",
        "冷渗含开口修正 K_inf′ = K_inf + (n_fan×23.52 + n_win×4·√A_win（洞口周长）)·0.24/0.86·1.2（小窗渗透线性化）".to_string(),
        format!(
            "{:.2} + ({:.0}×23.52 + {:.0}×4×{:.3}) = {:.1} + {:.1} = {:.1}；×0.24÷0.86×1.2 = {:.2}；Σ = {:.2} W/K",
            r.wk_infiltration,
            vn.fans_actual,
            vn.windows_actual,
            vn.window_area.sqrt(),
            vn.fans_actual * 23.52,
            vn.windows_actual * 4.0 * vn.window_area.sqrt(),
            aperture,
            aperture * 0.24 / 0.86 * 1.2,
            air_open
        ),
    );

    // ── 供暖 ──
    add(
        "",
        "房屋热负荷 Q_h = (K_wall′+K_floor+K_inf′)·Δt + K_roof·(Δt + 屋脊高/3.0)（高度梯度 0.33 K/m）".to_string(),
        format!(
            "Σ(A·K)′ = {:.1}+{:.1}+{:.1} = {:.1} W/K × {:.1} K = {:.0} W；\
屋面 {:.1}×({:.1}+{:.2}/3.0 = {:.2}) = {:.0} W；Q_h = {:.2} kW",
            wall_open,
            r.wk_floor,
            air_open,
            wall_open + r.wk_floor + air_open,
            dt,
            (wall_open + r.wk_floor + air_open) * dt,
            r.wk_roof,
            dt,
            h.roof_height,
            dt + h.roof_height / 3.0,
            r.wk_roof * (dt + h.roof_height / 3.0),
            hh.house_heat_load_kw
        ),
    );
    add(
        "",
        "热源单台能力 P_cap = f(t_o)（机组低温制热特性曲线）".to_string(),
        format!(
            "P({:.0} ℃) = {:.2} kW",
            ht.min_design_temp, hh.heating_capacity_kw
        ),
    );
    add(
        "",
        "所需机组 n_heat = round(Q_h/P_cap/0.92×10)/10".to_string(),
        format!(
            "{:.2}/{:.2}/0.92 = {:.2} → round×0.1 = {:.1} 台",
            hh.house_heat_load_kw,
            hh.heating_capacity_kw,
            hh.house_heat_load_kw / hh.heating_capacity_kw.max(1e-9) / 0.92,
            hh.heater_units_required
        ),
    );
    add(
        "",
        "外墙综合（ sol-air ）温度 t_sol = t_o + α墙·I×133/α_e（日辐射折平均强度）；屋面同式取 α屋"
            .to_string(),
        format!(
            "太阳附加 Δt_I = {:.2}×{:.1}×133/{:.0} = {:.2} K；墙：{:.1}+{:.2} = {:.2} ℃；屋面：α={:.2} → {:.2} ℃",
            cl.absorptivity,
            cl.solar_radiation,
            cl.alpha_e,
            cl.absorptivity * cl.solar_radiation * 133.0 / cl.alpha_e,
            cl.outdoor_temp,
            cl.absorptivity * cl.solar_radiation * 133.0 / cl.alpha_e,
            t_wall,
            cl.roof_absorptivity,
            crate::heat::wall_surface_temp(
                cl.outdoor_temp,
                cl.solar_radiation,
                cl.roof_absorptivity,
                cl.alpha_e
            )
        ),
    );
    add(
        "",
        "热泵性能指示 COP(t) = 单台制热量 P(t) ÷ 额定输入功率 P_in（指示值：低温端除霜未计入）"
            .to_string(),
        {
            let mm = crate::heat::find_heater_model(&ht.heater_model_name);
            let (cap, pw) = mm
                .map(|m| (m.capacity_kw(ht.min_design_temp), m.power_kw))
                .unwrap_or((0.0, 0.0));
            let cop = mm.map(|m| m.cop_at(ht.min_design_temp)).unwrap_or(0.0);
            format!(
                "COP({:.0} ℃) = {:.1} / {:.1} = {:.2}",
                ht.min_design_temp, cap, pw, cop
            )
        },
    );
    add(
        "",
        "末端单体散热能力 Φ_t = (t_ws − t_i)·g_t/1000（g_t = 单体散热系数 W/K）".to_string(),
        format!(
            "({:.0}−{:.0}) × {:.4} / 1000 = {:.4} kW",
            ht.water_temp, ht.target_room_temp, ht.terminal_unit_conductance, unit_power
        ),
    );
    add(
        "",
        "所需末端 n_term = ceil(Q_h/Φ_t/η_t)".to_string(),
        format!(
            "{:.2}/{:.4}/{:.2} = {:.0} → ceil = {:.0}",
            hh.house_heat_load_kw,
            unit_power,
            ht.terminal_efficiency,
            hh.house_heat_load_kw / unit_power.max(1e-9) / ht.terminal_efficiency,
            hh.terminal_count_required
        ),
    );
    add(
        "",
        "末端总热导 G_term = n_term,act·g_t/1000 [kW/K]".to_string(),
        format!(
            "{:.0} × {:.4} / 1000 = {:.2} kW/K",
            ht.terminal_count, ht.terminal_unit_conductance, hh.terminal_total_power_kw
        ),
    );
    add(
        "",
        "末端散热能力 = G_term × (t_ws − t_i) × η_t".to_string(),
        format!(
            "{:.2} × {:.1} × {:.2} = {:.1} kW",
            hh.terminal_total_power_kw,
            dtw,
            ht.terminal_efficiency,
            hh.terminal_total_power_kw * dtw * ht.terminal_efficiency
        ),
    );

    // ── 冷负荷与通风 ──
    add(
        "",
        "房屋冷负荷 Q_bld = ((t_sol,墙−25)·K_wall′ + (t_sol,屋−25)·K_roof + (t_o−25)(K_floor+K_inf′))/1000（墙面与屋面各按自己材料的 sol-air 温度）".to_string(),
        format!(
            "(({:.2}−25)×{:.1} + ({:.2}−25)×{:.1} + ({:.1}−25)×({:.1}+{:.1}))/1000 = {:.2} kW",
            t_wall,
            wall_open,
            crate::heat::wall_surface_temp(
                cl.outdoor_temp,
                cl.solar_radiation,
                cl.roof_absorptivity,
                cl.alpha_e
            ),
            r.wk_roof,
            cl.outdoor_temp,
            r.wk_floor,
            air_open,
            v.house_cool_load_kw
        ),
    );
    add(
        "",
        "最低维持通风量 L_min = q_CO<sub>2</sub>·体重·只数".to_string(),
        format!(
            "{:.2} × {:.2} × {:.0} = {:.0} m³/h",
            b.co2_respiration_vol, b.breed_weight, b.breed_vol, b.minimum_ventilation
        ),
    );
    add(
        "",
        "生物呼吸 ATP 热 = 风量×0.03%×1000/6 耗氧(L/h)×2870 J/L×0.6÷3600÷22.4".to_string(),
        format!(
            "耗氧 O₂ = {:.0}×0.03%×1000/6 = {:.1} L/h；Q = {:.1}×2870×0.6÷3600÷22.4 = {:.2} kW",
            b.minimum_ventilation,
            b.minimum_ventilation * 0.0003 * 1000.0 / 6.0,
            b.minimum_ventilation * 0.0003 * 1000.0 / 6.0,
            v.bio_cool_load_kw
        ),
    );
    add(
        "",
        "总冷负荷 Q_base = Q_bld + Q_bio + Q_eq".to_string(),
        format!(
            "{:.2} + {:.2} + {:.1} = {:.2} kW",
            v.house_cool_load_kw,
            v.bio_cool_load_kw,
            vn.equipment_cool_load_kw,
            v.total_cool_load_kw
        ),
    );
    add(
        "",
            "湿帘物理模型（7090 厂商曲线）：蒸发冷却效率 ε 按厂商饱和度曲线取值（面风速 1.6 m/s），送风 t_sup = t_o − ε·(t_o−τ)（τ=室外湿球）；含湿量沿进风等焓线反解；帘空气侧压降 Δp 按厂商压降曲线查取（风机静压校核）；舍内目标 t_i = max(18, t_sup+0.4) ℃".to_string(),
        format!(
            "ε = {:.1}%（厂商曲线，v=1.6 m/s）→ t_sup = {:.2} − {:.3}×({:.2}−{:.2}) = {:.2} ℃；\
t_i = max(18, {:.2}+0.4) = {:.2} ℃，Δt = {:.2} K；Δp = {:.0} Pa",
            c.saturation_pct,
            cl.outdoor_temp,
            c.saturation_pct / 100.0,
            cl.outdoor_temp,
            crate::psychro::calc_from_rel_hum(
                cl.outdoor_temp,
                (cl.humidity_pct / 100.0).clamp(0.05, 1.0),
                101_325.0,
            )
            .t_wet_bulb,
            c.supply_temperature,
            c.supply_temperature,
            c.pad_target,
            (c.pad_target - c.supply_temperature).max(0.3),
            c.pressure_drop_pa
        ),
    );
    add(
        "",
        "计算通风量 L_v = 1.55·Q_base·1000 / (ρ·cₚ·(t_i−t_sup))（显热平衡+安全系数；Δt 基准 0.4 K）"
            .to_string(),
        format!(
            "分子 1.55×{:.2}×1000 = {:.0}；分母 1.2×1.006×{:.2} = {:.4}；L_v = {:.0} ÷ {:.4} = {:.0} m³/h{}",
            v.total_cool_load_kw,
            1.55 * v.total_cool_load_kw * 1000.0,
            (c.pad_target - c.supply_temperature).max(0.3),
            1.2 * 1.006 * (c.pad_target - c.supply_temperature).max(0.3),
            1.55 * v.total_cool_load_kw * 1000.0,
            1.2 * 1.006 * (c.pad_target - c.supply_temperature).max(0.3),
            c.air_volume,
            if c.dt_used > 0.41 {
                format!("（Δt = {:.2} K）", c.dt_used)
            } else {
                String::new()
            }
        ),
    );
    add(
        "",
        if v.aero_based {
            "计算风机数 n_fan = ceil(L_v/q_fan,op) + n_spare——q_fan,op 为**空气动力学工作点**风量：\
厂商分段线性 P-Q 样本曲线 Q(ΔP)；系统静压 ΔP_sys = 帘润湿压降\
——按实配运行台数总风量对系统曲线 dp(Q) = Δp×(Q/L_v)² 反校\
（湿帘降温与小窗通风互斥，工作点由帘工况决定）；\
可变效率 η_op = Q(ΔP_sys)/Q(0 Pa 自由空气量) 透传".to_string()
        } else {
            "计算风机数 n_fan = ceil(L_v/(q_fan·η_fan)) + n_spare".to_string()
        },
        {
            let hood_f = if vn.fan_light_hood { 0.91 } else { 1.0 };
            if v.aero_based {
                let q_curve = v.fan_q_operating / hood_f;
                let q_ref = v.fan_q_operating / v.fan_eta_operating.max(1e-6);
                // 自定义基准注记：−20Pa 校核点风量与机型样本不一致时说明缩放
                let custom_note =
                    match crate::vent::aero_curve_for(&format!(
                        "{} {}",
                        vn.fan_category, vn.fan_model_name
                    )) {
                        Some(cc) => {
                            let q20 = cc.q_at(20.0);
                            if q20 > 0.0 && (q20 - vn.fan_unit_air_volume).abs() > 1.0 {
                                format!(
                                    "；<b>自定义风机</b>：曲线形状沿用机型样本，−20 Pa 基准缩放至 {:.0}（×{:.3}）",
                                    vn.fan_unit_air_volume,
                                    vn.fan_unit_air_volume / q20
                                )
                            } else {
                                String::new()
                            }
                        }
                        None => String::new(),
                    };
                let n_run = (c.air_volume / v.fan_q_operating.max(1.0)).ceil();
                let q_tot = n_run * v.fan_q_operating;
                format!(
                    "运行台数反校：n_run = ⌈{:.0}/{:.0}⌉ = {:.0} 台 → Q_tot = {:.0} m³/h；\
ΔP_sys = 帘润湿压降 {:.1}×({:.0}/{:.0})² = {:.1} Pa\
（湿帘降温与小窗通风互斥，工作点由帘工况决定）；\
工作点 Q({:.1}) = {:.0}{}；\
η_op = {:.0}/{:.0} = {:.2}；\
{:.0}/{:.0} = {:.2} → ⌈{:.2}⌉ = {:.0} + 备用 {:.0} = {:.0} 台{}",
                    c.air_volume,
                    v.fan_q_operating,
                    n_run,
                    q_tot,
                    c.pressure_drop_pa,
                    q_tot,
                    c.air_volume,
                    c.pressure_drop_pa * (q_tot / c.air_volume.max(1.0)).powi(2),
                    v.dp_system_pa,
                    q_curve,
                    if vn.fan_light_hood {
                        format!(" ×0.91（遮光罩风阻）= {:.0}", v.fan_q_operating)
                    } else {
                        format!(" = {:.0}", v.fan_q_operating)
                    },
                    v.fan_q_operating,
                    q_ref,
                    v.fan_eta_operating,
                    c.air_volume,
                    v.fan_q_operating,
                    c.air_volume / v.fan_q_operating.max(1e-6),
                    (c.air_volume / v.fan_q_operating.max(1e-6)).ceil(),
                    (c.air_volume / v.fan_q_operating.max(1e-6)).ceil(),
                    vn.fans_spare,
                    v.fans_required,
                    custom_note
                )
            } else {
                let raw = c.air_volume
                    / (vn.fan_unit_air_volume * vn.fan_efficiency * hood_f).max(1e-9);
                format!(
                    "{:.0}/({:.0}×{:.2}{} = {:.1}) = {:.2} → ⌈{:.2}⌉ = {:.0} + 备用 {:.0} = {:.0} 台",
                    c.air_volume,
                    vn.fan_unit_air_volume,
                    vn.fan_efficiency,
                    if vn.fan_light_hood { "×0.91（遮光罩）" } else { "" },
                    vn.fan_unit_air_volume * vn.fan_efficiency * hood_f,
                    raw,
                    raw,
                    raw.ceil(),
                    vn.fans_spare,
                    v.fans_required
                )
            }
        },
    );
    add(
        "",
        "GB 规范湿帘面积 A_pad,GB = Q_base/(1.03·1.2·v_face·Δt_eff)；\
Δt_eff = clamp(t_o − t_sup, 0.9, 1.2)（穿帘干球温降，即 ε_帘×室外湿球温差）".to_string(),
        {
            // 与引擎同一校准式：Δt_eff = clamp(t_o − t_sup, 0.9, 1.2)
            let dt_eff = (cl.outdoor_temp - c.supply_temperature).clamp(0.9, 1.2);
            format!(
                "Δt_eff = clamp({:.2} − {:.2} → 0.9~1.2) = {:.2} K；\
{:.2}/1.03/1.2/1.6/{:.2} = {:.2} m²",
                cl.outdoor_temp,
                c.supply_temperature,
                dt_eff,
                v.total_cool_load_kw,
                dt_eff,
                c.pad_area_gb
            )
        },
    );
    add(
        "",
        "平衡法湿帘面积 A_pad = L_v/3600/1.6".to_string(),
        format!(
            "{:.0}/3600/1.6 = {:.2} m²",
            c.air_volume, c.pad_area_balance
        ),
    );
    add(
        "",
        "装机湿帘（额定抽吸）A_pad,act = ⌈n_fan,act × 0.9⌉·q_fan/3600/1.6，按 0.6 m 模块取整——实配含 10% 备用风机（×0.9 折算运行台数并取整），备用机不常开、不计入风量；q_fan 取 −20 Pa 校核点额定风量".to_string(),
        format!(
            "⌈{:.0}×0.9⌉={:.0} 台 × {:.0}/3600/1.6 = {:.1} → {:.1} m²",
            vn.fans_actual,
            (vn.fans_actual * 0.9).ceil(),
            vn.fan_unit_air_volume,
            (vn.fans_actual * 0.9).ceil() * vn.fan_unit_air_volume / 3600.0 / 1.6,
            vn.pad_area_actual
        ),
    );
    add(
        "",
        "湿帘补水量 M_w = E_帘×1.15 + 0.02·A_pad——E_帘（湿帘总蒸发能力，7090 传质法，按 <b>60 % 标准相对湿度</b>常规工况计算，不随气候设计湿度联动）为补水主体：蒸发的水随排风离开循环系统，必须补足；×1.15 计入【排污】：循环水蒸发后盐分浓缩，须定期排掉部分浓水防止湿帘与管路结垢——与【飘失】：穿帘气流夹带细小水滴被吹出；0.02·A_pad 为润湿面滴漏损失。蒸发吸热 Φ_ev = W_bio·r/3600（湿度指标另按 W_bio 水分平衡校核）".to_string(),
        format!(
            "M_w = (E_帘×1.15 + 0.02×A_pad)/1000 = ({:.2}×1.15 + 0.02×{:.1})/1000 = ({:.1}+{:.2})/1000 = {:.1}/1000 = {:.2} m³/h；Φ_ev = {:.2} kW",
            c.evap_capacity_kg_h,
            vn.pad_area_actual,
            c.evap_capacity_kg_h * 1.15,
            0.02 * vn.pad_area_actual,
            c.evap_capacity_kg_h * 1.15 + 0.02 * vn.pad_area_actual,
            c.water_use,
            c.cooling_power_kw
        ),
    );
    add(
        "",
        "体感温度指数 THI = t − 0.55(1−p_d(t_dew)/p_d(t))(t−14) − 0.65·v − 2.7（p_d=饱和水蒸气压；含走风降温项的禽舍生产指标）".to_string(),
        {
            // 口径提示：实现以室外湿球 τ 为气温项（湿帘近饱和工况）
            let tau = crate::psychro::calc_from_rel_hum(
                cl.outdoor_temp,
                (cl.humidity_pct / 100.0).clamp(0.05, 1.0),
                101_325.0,
            )
            .t_wet_bulb;
            let hum_term =
                tau - 0.65 * c.pad_wind_speed - 2.7 - c.apparent_temperature;
            format!(
                "t = τ_o = {:.1} ℃（湿球基准）：{:.1} − 0.55(1−p_d比)×({:.1}−14) − 0.65×{:.2} − 2.7；湿度项 = {:.2} → {:.2} ℃",
                tau,
                tau,
                tau,
                c.pad_wind_speed,
                hum_term,
                c.apparent_temperature
            )
        }
    );
    add(
        "",
        "舍内目标温度 t_i = max(18, t_sup+0.4) ℃（物理下限＝送风+0.4 K，贴送风换低温）".to_string(),
        {
            let t_wb = crate::psychro::calc_from_rel_hum(
                cl.outdoor_temp,
                (cl.humidity_pct / 100.0).clamp(0.05, 1.0),
                101_325.0,
            )
            .t_wet_bulb;
            format!(
                "max(18, {:.1}+0.4) = {:.2} ℃（τ_o = {:.1} ℃ 参考，Δt = {:.1} K）",
                c.supply_temperature,
                c.room_temperature,
                t_wb,
                (c.room_temperature - c.supply_temperature).max(0.3)
            )
        },
    );

    // ── 小窗 ──
    add(
        "",
        "洞口当量直径 d<sub>0</sub> = √(4·A_win/π)；射流衰减系数 φ = 0.48/(0.076·(舍宽/2)/d<sub>0</sub> + 0.147)".to_string(),
        format!(
            "d<sub>0</sub> = √(4×{:.4}/π) = √{:.4} = {:.3} m；φ = 0.48/(0.076×{:.1}/{:.3}+0.147) = {:.3}；\
v<sub>0</sub> = max(0.6/{:.3}, 2.2) = {:.2} m/s",
            vn.window_area,
            4.0 * vn.window_area / std::f64::consts::PI,
            d0,
            h.width / 2.0,
            d0,
            k_jet,
            k_jet,
            vn.window_speed
        ),
    );
    add(
        "",
        "最低小窗数 n_min = ceil(L_min/(A_win×1 m/s×3600))（夜间最小通风口径）".to_string(),
        format!(
            "{:.0}/({:.4}×1×3600 = {:.1}) = {:.2} → ceil = {:.0} 个",
            b.minimum_ventilation,
            vn.window_area,
            vn.window_area * 3600.0,
            b.minimum_ventilation / (vn.window_area * 3600.0).max(1e-9),
            v.windows_min_count
        ),
    );
    add(
        "",
        "CO<sub>2</sub> 2000PPM 稀释风量 = 呼吸量·体重·只数 按 (mg<sub>5</sub><sub>0</sub><sub>0</sub><sub>0</sub>−mg<sub>2</sub><sub>0</sub><sub>0</sub><sub>0</sub>)/(mg<sub>2</sub><sub>0</sub><sub>0</sub><sub>0</sub>−mg<sub>4</sub><sub>0</sub><sub>0</sub>) 折算".to_string(),
        format!(
            "基础呼吸风量 L_min = {:.2}×{:.2}×{:.0} = {:.0}；折算系数 (5000−2000)/(2000−400) = 1.875；L_2000 = {:.0}×1.875 = {:.0} m³/h",
            b.co2_respiration_vol,
            b.breed_weight,
            b.breed_vol,
            b.minimum_ventilation,
            b.minimum_ventilation,
            b.co2_2000ppm_vol
        ),
    );
    add(
        "",
        "空间修正小窗数 n_am = ceil(L_2000/(A_win×v<sub>0</sub>×3600))".to_string(),
        format!(
            "{:.0}/({:.4}×{:.2}×3600 = {:.1}) = {:.2} → ceil = {:.0} 个",
            b.co2_2000ppm_vol,
            vn.window_area * vn.window_speed * 3600.0,
            vn.window_area,
            vn.window_speed,
            b.co2_2000ppm_vol / (vn.window_area * vn.window_speed * 3600.0).max(1e-9),
            v.windows_amended_count
        ),
    );
    add(
        "",
        "最优小窗数 n_opt：2000PPM 风量对齐风机档 ⌈L_v/(q_fan·η_fan)⌉×(q_fan·η_fan)，再折算窗数向上取偶（≥n_min）".to_string(),
        {
            // 完整过程：档位 → 对齐风量 → 折窗 → 取偶 → 与 n_min 比较
            let steps = (b.co2_2000ppm_vol / fan_ep).ceil();
            let aligned = steps * fan_ep;
            let conv = aligned / (vn.window_area * vn.window_speed * 3600.0).max(1e-9);
            let conv_up = conv.ceil();
            let even = (conv_up / 2.0).ceil() * 2.0;
            format!(
                "档位 = ⌈{:.0}/{:.0}⌉ = {:.0} → 对齐风量 {:.0}×{:.0} = {:.0} m³/h；\
窗数 = ⌈{:.0}/({:.4}×{:.2}×3600)⌉ = ⌈{:.1}⌉ = {:.0} → 取偶 {:.0}；\
max(取偶, n_min {:.0}) → {:.0} 个",
                b.co2_2000ppm_vol,
                fan_ep,
                steps,
                steps,
                fan_ep,
                aligned,
                aligned,
                vn.window_area,
                vn.window_speed,
                conv,
                conv_up,
                even,
                v.windows_min_count,
                v.windows_best_count
            )
        },
    );
    add(
        "",
        "计算小窗总数 n_tot = max(取偶(n_am×0.2 + n_opt×0.9)，取偶(⌈舍长/(窗宽+2.0)⌉×2))"
            .to_string(),
        {
            let weighted = v.windows_amended_count * 0.2 + v.windows_best_count * 0.9;
            let by_weight = (weighted / 2.0).ceil() * 2.0;
            let by_spacing = (h.length / (vn.window_width_m + 2.0)).ceil() * 2.0;
            format!(
                "加权 = {:.0}×0.2+{:.0}×0.9 = {:.1} → 取偶 {:.0}；\
间距下限 = ⌈{:.1}/({:.2}+2.0)⌉×2 = {:.0}；max({:.0}, {:.0}) → {:.0} 个",
                v.windows_amended_count,
                v.windows_best_count,
                weighted,
                by_weight,
                h.length,
                vn.window_width_m,
                by_spacing,
                by_weight,
                by_spacing,
                v.windows_total_count
            )
        },
    );
    add(
        "",
        "小窗净间距 = 舍长/⌈n_win/2⌉ − 窗宽（要求 1.2 ~ 2.0 m）".to_string(),
        format!(
            "⌈{:.0}/2⌉ = {:.0} 列/侧；{:.1}/{:.0} = {:.2} − {:.2} = {:.2} m",
            vn.windows_actual,
            (vn.windows_actual / 2.0).ceil(),
            h.length,
            (vn.windows_actual / 2.0).ceil(),
            h.length / (vn.windows_actual / 2.0).ceil(),
            vn.window_width_m,
            h.length / (vn.windows_actual / 2.0).ceil() - vn.window_width_m
        ),
    );

    // ── 给排水与冗余 ──
    add(
        "",
        "日采食量 = 采食标准(按种类体重档) × 只数".to_string(),
        format!(
            "{:.3} × {:.0} = {:.0} kg/日",
            breed_feed_kg(b.breed_type),
            b.breed_vol,
            breed_feed_kg(b.breed_type) * b.breed_vol
        ),
    );
    add(
        "",
        "突发用水量 Q_突发 = 日采食 ÷ 3 × 水料比 2.2 / 1000".to_string(),
        format!(
            "{:.0}÷3 = {:.1} ×2.2 = {:.1} ÷1000 = {:.2} m³/h",
            breed_feed_kg(b.breed_type) * b.breed_vol,
            breed_feed_kg(b.breed_type) * b.breed_vol / 3.0,
            breed_feed_kg(b.breed_type) * b.breed_vol / 3.0 * 2.2,
            breed_feed_kg(b.breed_type) * b.breed_vol / 3.0 * 2.2 / 1000.0
        ),
    );
    add(
        "",
        "水源选型：流量需求 Q_w = Q_突发 + E_帘（实际合成流量直加；E_帘=湿帘总蒸发能力，7090 传质法全湿润口径）；管径 DN = min{D | v = Q_w/3600/(πD²/4) ≤ 1.5 m/s}（GB 50015 经济流速上限）".to_string(),
        {
            let sudden_w = breed_feed_kg(b.breed_type) * b.breed_vol / 3.0 * 2.2 / 1000.0;
            let evap_cap = c.evap_capacity_kg_h / 1000.0;
            let (wq, wdn, wv) = crate::bio::water_supply_sizing(sudden_w, evap_cap);
            format!(
                "{:.2} + {:.2} = {:.2} m³/h → 选 DN{:.0}：v = {:.2}/3600 ÷ (π×({:.0}/1000)²/4) = {:.2} m/s ≤ 1.5",
                sudden_w,
                evap_cap,
                wq,
                wdn,
                wq,
                wdn,
                wv
            )
        },
    );
    add(
        "",
        "热网运行点：Q_src = 实配机组 × 单台能力 P(t_o) × fit × 0.92；末端-围护平衡 t_eq = (0.92G·t_ws + K·t_o + 屋面梯度)/(K + 0.92G)；Q_src ≥ 平衡需求 → 舍温 = t_eq（饱和＝可达温度），否则 舍温 = t_o + Q_src/K、运行供水 = 舍温 + Q_src/0.92G（台数↑ 舍温/水温/末端散热同升）".to_string(),
        format!(
            "Q_src = 实配 × 单台 {:.2} × fit × 0.92 = {:.0} kW；热网平衡舍温 {:.2} ℃{}可达上限 {:.2} ℃{}",
            hh.heating_capacity_kw,
            hh.network_q_src_kw,
            hh.network_room_temp,
            if hh.network_heat_limited { " < " } else { " = " },
            hh.reachable_temp,
            if hh.network_heat_limited {
                "（热源受限，加机组可提升）"
            } else {
                "（机组充足，饱和于末端-围护平衡）"
            }
        ),
    );
    add(
        "",
        "舍内湿度指标 φ_i：含湿量水分平衡 d_i = d_sup + W_bio/(ρ·L_v)，φ_i = φ(t_i, d_i)（湿度为校核指标而非风量约束）".to_string(),
        format!(
            "W_bio = {:.1}×0.55/24 = {:.2} kg/h；Δd = W_bio/(1.2×{:.0}) 沿等焓线 → φ_i = {:.1} %（t_i = {:.1} ℃）",
            b.breed_need_water,
            b.breed_need_water * 0.55 / 24.0,
            c.air_volume,
            c.solved_humidity.unwrap_or(0.0),
            c.room_temperature
        ),
    );
    add(
        "",
        "净吸热系数 s = Q_base / L_v（单位通风量的净供冷量，显示指标）".to_string(),
        format!(
            "{:.2} kW / {:.0} m³/h = {:.4} kW/(m³/h)",
            v.total_cool_load_kw, c.air_volume, c.s_tf
        ),
    );
    add(
        "",
        "湿帘总蒸发能力 E_帘（7090 传质法，模块 600 宽×实际高×帘厚）：单模块 W = β·(p_s(τ)−φ₀·p_s(t_o))·A_eq·(1.013/P̄)，β=(α+0.00363·v)/10⁵，A_eq 按波高7×波纹角45° 展开；E_帘 = W × ⌈A_pad,act ÷ (0.6×高)⌉（水侧=湿球 τ、气侧=室外进气、v=1.6 m/s）".to_string(),
        format!(
            "模块数 = ⌈{:.1}/(0.6×{:.2})⌉ = ⌈{:.2}⌉ = {:.0} 块；\
单模块 W = {:.2}/ {:.0} = {:.2} kg/h；E_帘 = {:.2}×{:.0} = {:.2} kg/h ÷ 1000 = {:.2} m³/h",
            vn.pad_area_actual,
            vn.pad_side_height.max(0.3),
            vn.pad_area_actual / (0.6 * vn.pad_side_height.max(0.3)),
            (vn.pad_area_actual / (0.6 * vn.pad_side_height.max(0.3))).ceil(),
            c.evap_capacity_kg_h,
            (vn.pad_area_actual / (0.6 * vn.pad_side_height.max(0.3))).ceil(),
            c.evap_capacity_kg_h / (vn.pad_area_actual / (0.6 * vn.pad_side_height.max(0.3))).ceil(),
            c.evap_capacity_kg_h / (vn.pad_area_actual / (0.6 * vn.pad_side_height.max(0.3))).ceil(),
            (vn.pad_area_actual / (0.6 * vn.pad_side_height.max(0.3))).ceil(),
            c.evap_capacity_kg_h,
            c.evap_capacity_kg_h / 1000.0
        ),
    );
    add(
        "",
        "降温效率 ε = (t_o − t_sup) ÷ (t_o − τ) × 100%（湿帘湿冷却效率定义；τ=室外湿球）"
            .to_string(),
        {
            use crate::psychro::calc_from_rel_hum;
            let tw = calc_from_rel_hum(
                cl.outdoor_temp,
                (cl.humidity_pct / 100.0).clamp(0.05, 1.0),
                101_325.0,
            )
            .t_wet_bulb;
            let e = if (cl.outdoor_temp - tw).abs() > 0.01 {
                ((cl.outdoor_temp - c.supply_temperature) / (cl.outdoor_temp - tw) * 100.0)
                    .clamp(0.0, 100.0)
            } else {
                100.0
            };
            format!(
                "({:.1} − {:.1}) ÷ ({:.1} − {:.1}) = {:.1} %",
                cl.outdoor_temp, c.supply_temperature, cl.outdoor_temp, tw, e
            )
        },
    );
    add(
        "",
        "实配跟随（自动配置开启时）= ⌊计算值×1.1⌋（计算值＝工作点风量口径），小窗 = ceil(计算总数)——含 10% 冗余"
            .to_string(),
        format!(
            "跟随口径：风机 ⌊{:.0}×1.1⌋ = {:.0}，小窗 ⌈{:.0}⌉ = {:.0}；实配（可手动调整）：风机 {:.0} 台、末端 {:.0}、小窗 {:.0} 个",
            v.fans_required,
            (v.fans_required * 1.1).floor(),
            v.windows_total_count,
            v.windows_total_count.ceil(),
            vn.fans_actual,
            ht.terminal_count,
            vn.windows_actual
        ),
    );

    let mut o = String::from(if keep.is_some() {
        // 热负荷册符号表（仅采暖链符号）
        "<table><tr><th>符号</th><th>含义</th><th>符号</th><th>含义</th></tr>\
<tr><td>Lₛ</td><td>屋面坡长 m</td><td>V</td><td>房间容积 m³</td></tr>\
<tr><td>A_long / A_gable</td><td>纵墙 / 山墙面积 m²</td><td>A_roof / A_floor</td><td>屋面 / 地面面积 m²</td></tr>\
<tr><td>A_base</td><td>勒脚带面积 m²</td><td>U_w / U_r</td><td>墙 / 屋面传热系数 W/(m²·K)</td></tr>\
<tr><td>K_wall / K_roof / K_floor / K_inf</td><td>墙 / 屋面 / 地面 / 渗透热损系数 W/K（′=含开口修正）</td><td>Δt</td><td>设计温差 = 室内 − 室外 K</td></tr>\
<tr><td>Q_h</td><td>房屋热负荷 kW</td><td>P_cap / n_heat</td><td>热源单台能力 kW / 机组数</td></tr>\
<tr><td>COP(t)</td><td>热源性能指示（制热 ÷ 输入功率）</td><td>Σ(A·K)</td><td>总单位热损 W/K</td></tr>\
<tr><td>g_t / G_term / Φ_t / η_t</td><td>末端单体热导 W/K / 总热导 kW/K / 单体散热 kW / 效率</td><td>t_o / t_i / t_ws</td><td>室外 / 室内 / 供水温度 ℃</td></tr>\
</table>"
    } else {
        "<table><tr><th>符号</th><th>含义</th><th>符号</th><th>含义</th></tr>\
<tr><td>τ_o</td><td>室外湿球温度 ℃</td><td>t_sup / φ_sup</td><td>湿帘送风温度 ℃ / 送风相对湿度 %（等焓线交点）</td></tr>
<tr><td>W_bio</td><td>舍内水分产生 kg/h（日饮水×0.55/24）</td><td>φ_i</td><td>舍内相对湿度 %（水分平衡指标）</td></tr>
<tr><td>t_o / t_i / t_ws</td><td>室外 / 室内 / 供水温度 ℃</td><td>s</td><td>净吸热系数 Q_base/L_v kW/(m³/h)</td></tr>\
<tr><td>Lₛ</td><td>屋面坡长 m</td><td>V</td><td>房间容积 m³</td></tr>\
<tr><td>A_long / A_gable</td><td>纵墙 / 山墙面积 m²</td><td>A_roof / A_floor</td><td>屋面 / 地面面积 m²</td></tr>\
<tr><td>A_base</td><td>勒脚带面积 m²</td><td>A_win</td><td>单个洞口面积 m²</td></tr>\
<tr><td>U_w / U_r</td><td>墙 / 屋面传热系数 W/(m²·K)</td><td>K_wall / K_roof / K_floor / K_inf</td><td>墙 / 屋面 / 地面 / 渗透热损系数 W/K（′=含开口修正）</td></tr>\
<tr><td>Q_h</td><td>房屋热负荷 kW</td><td>Q_base</td><td>总冷负荷 kW（分项 Q_bld / Q_bio / Q_eq）</td></tr>\
<tr><td>L_v / L_min</td><td>计算 / 最低通风量 m³/h</td><td>q_fan / η_fan / n_fan</td><td>单台额定风量 / 效率 / 风机数</td></tr>\
<tr><td>n_min / n_am / n_opt / n_tot / n_win</td><td>小窗 最低 / 修正 / 最优 / 总数 / 实配 个</td><td>v<sub>0</sub> / d<sub>0</sub> / φ</td><td>洞口风速 / 当量直径 / 射流衰减系数</td></tr>\
<tr><td>v_pad</td><td>过帘风速 m/s</td><td>s = s_lat + s_sens</td><td>湿帘净吸热系数 kW/(m³/h)（潜热+显热分量）</td></tr>\
<tr><td>β / Δp_v / r</td><td>传质系数 / 水汽压差 / 汽化潜热</td><td>M_w / Φ_ev</td><td>蒸发需水量 m³/h（60% RH） / 蒸发吸热 kW</td></tr>\
<tr><td>THI</td><td>体感温度指数</td><td>P_cap / n_heat</td><td>热源单台能力 kW / 机组数</td></tr>\
<tr><td>g_t / G_term / Φ_t / η_t</td><td>末端单体热导 W/K / 总热导 kW/K / 单体散热 kW / 效率</td><td>Q_w / DN</td><td>水源选型流量 m³/h / 公称管径 mm</td></tr>\
</table>"
    });
    o += "<table><tr><th>序号</th><th>公式</th><th>代入与结果</th></tr>";
    for (i, (_num, f, res)) in items.iter().enumerate() {
        let n = i + 1;
        o += &format!("<tr><td><a name=\"eq{n}\">（{n}）</a></td><td>{f}</td><td>{res}</td></tr>");
    }
    o += "</table>";
    let eqs: Vec<(String, usize)> = items
        .iter()
        .enumerate()
        .map(|(i, (_, f, _))| (f.clone(), i + 1))
        .collect();
    (o, eqs)
}

/// 表格行的公式引用上标：按关键词在附录公式中检索首个匹配，生成
/// `<sup>[<a href="#eqN">N</a>]</sup>`（Word/WPS 内跳转书签）
fn eqsup(eqs: &[(String, usize)], keys: &[&str]) -> String {
    let mut out = String::new();
    for k in keys {
        if let Some((_, n)) = eqs.iter().find(|(f, _)| f.contains(k)) {
            out += &format!("<sup>[<a href=\"#eq{n}\">{n}</a>]</sup>");
        }
    }
    out
}

fn breed_cn(t: crate::data::BreedType) -> &'static str {
    use crate::data::BreedType::*;
    match t {
        LayerChicken => "蛋鸡",
        BroilerChicken => "肉鸡",
        LayerDuck => "蛋鸭",
        BroilerDuck => "肉鸭",
        Goose => "鹅",
        Other => "其他",
    }
}

fn breed_feed_kg(t: crate::data::BreedType) -> f64 {
    use crate::data::BreedType::*;
    match t {
        LayerChicken => 0.115,
        BroilerChicken => 0.17,
        LayerDuck => 0.13,
        BroilerDuck => 0.19,
        Goose => 0.22,
        Other => 0.15,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::compute_all;

    #[test]
    fn civil_date_sane() {
        // 2026-01-01 = 20454 天
        assert_eq!(civil_from_days(20_454), (2026, 1, 1));
    }

    #[test]
    fn sm3_vectors() {
        // GB/T 32905-2016 附录 A 标准测试向量
        assert_eq!(
            sm3_hex(b"abc"),
            "66c7f0f462eeedd9d1f2d46bdc10e4e24167c4875cf2f7a2297da02b8f4ba8e0"
        );
        assert_eq!(
            sm3_hex(&[b'a', b'b', b'c', b'd'].repeat(16)),
            "debe9ff92275b8a138604889c18e5a4d6fdb70e5387e5765293dcba39c0c5732"
        );
    }

    #[test]
    fn pdf_sidecar_roundtrip() {
        let dir = std::env::temp_dir().join("duckcm_sm3_test");
        std::fs::create_dir_all(&dir).unwrap();
        let fake = dir.join("t.pdf");
        std::fs::write(&fake, b"PDF-ORIGINAL").unwrap();
        write_pdf_sm3(&fake).unwrap();
        assert_eq!(verify_pdf_sm3(&fake), Ok(true));
        std::fs::write(&fake, b"PDF-TAMPERED!").unwrap();
        assert_eq!(verify_pdf_sm3(&fake), Ok(false));
        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn verify_fingerprint_roundtrip() {
        let body = "<h2>一、测试</h2><p>正文数值 3.14。</p>";
        let doc = format!("{body}{}", sig_block(body));
        let html = format!("<div class=WordSection1><!-- 测试书 -->{doc}</div></body></html>");
        assert_eq!(verify_doc_fingerprint(&html), Ok(true));
        // 篡改一个数字 → 失配
        let bad = html.replace("3.14", "3.15");
        assert_eq!(verify_doc_fingerprint(&bad), Ok(false));
        // 非计算书 → Err
        assert!(verify_doc_fingerprint("<html>hi</html>").is_err());
    }

    #[test]
    fn code128_png_base64_sane() {
        // START B 模式 "211214" → 模块 11 01 001 0 0000? 首 6 个模块应为 11010010000
        let bits = code128b_bits("0");
        assert_eq!(&bits[0..11], &[1, 1, 0, 1, 0, 0, 1, 0, 0, 0, 0][..]);
        // 1 数据符：start(11)+data(11)+check(11)+stop(13) = 46 模块
        assert_eq!(bits.len(), 46);
        // PNG 魔数与 base64 已知向量
        let (w, h, px) = code128_gray("ab12", 2);
        let png = png_gray8(w, h, &px);
        assert_eq!(&png[0..8], b"\x89PNG\r\n\x1a\n");
        assert_eq!(base64(b"Man"), "TWFu");
        assert_eq!(base64(b"a"), "YQ==");
    }

    /// 开发通道：`cargo test -- --ignored gen_current` ——
    /// 读取实际工程 json（state + curve 同 GUI 载入路径）生成两份计算书，
    /// 用于无法操作 GUI 时验证文案（与「生成」按钮同一代码路径 write_reports）。
    #[test]
    #[ignore]
    fn gen_current_project_books() {
        let dir = std::path::Path::new(env!("USERPROFILE")).join("Documents\\DuckCM");
        let path = dir.join("未命名工程.json");
        let raw = std::fs::read_to_string(&path).expect("工程文件");
        let v: serde_json::Value = serde_json::from_str(&raw).unwrap();
        let state: ProjectState = serde_json::from_value(v["state"].clone()).unwrap();
        let cv = &v["curve"];
        let curve = crate::heat::HeatingCurve {
            coeff: cv["coeff"]
                .as_array()
                .map(|a| a.iter().filter_map(|x| x.as_f64()).collect())
                .unwrap_or_default(),
            terminal_count: cv["terminal_count"].as_f64().unwrap_or(0.0),
            unit_energy_w_per_k: cv["unit_energy_w_per_k"].as_f64().unwrap_or(0.0),
            water_temp: cv["water_temp"].as_f64().unwrap_or(50.0),
            units: cv["units"].as_f64().unwrap_or(1.0),
            fit: cv["fit"].as_f64().unwrap_or(0.93),
        };
        let units = curve.units;
        // 构造层描述：新 json 为 Layer 数组 → 「75mm 玻璃丝棉 + …」；旧版为字符串
        let desc = |k: &str| -> String {
            if let Ok(ls) = serde_json::from_value::<Vec<crate::material::Layer>>(v[k].clone()) {
                if !ls.is_empty() {
                    return ls
                        .iter()
                        .map(|l| format!("{:.0}mm {}", l.thickness_mm, l.name))
                        .collect::<Vec<_>>()
                        .join(" + ");
                }
            }
            v[k].as_str().unwrap_or("75mm 玻璃丝棉").to_string()
        };
        let wall = desc("wall_layers");
        let roof = desc("roof_layers");
        let out = crate::compute_all(&state, Some(&curve));
        let (a, b) = write_reports(&state, &out, units, &path.to_string_lossy(), &wall, &roof)
            .expect("生成失败");
        println!("已生成: {a}\n         {b}");
    }

    #[test]
    fn enlex_translates_key_terms() {
        use crate::enlex::{translate_en, translate_en_with};
        // 专名保护：工程名中的育种词不被翻译，独立育种词照常翻译
        let mixed = translate_en_with("三穗蛋鸭育雏 · 蛋鸭", &["三穗蛋鸭育雏"]);
        assert!(mixed.contains("三穗蛋鸭育雏"), "工程名被误译: {mixed}");
        assert!(mixed.contains("Layer duck"), "独立育种词未译: {mixed}");
        assert!(translate_en("推拉风机").contains("Push-pull exhaust fan"));
        assert!(translate_en("20mm地暖管").contains("Underfloor heating pipe"));
        assert!(translate_en("白色彩钢（α=0.35）").contains("White colour-coated steel"));
        assert!(translate_en("九、结论").contains("IX. Conclusion"));
        assert!(translate_en("热源受限：").contains("Heat source limited"));
        // 纯 ASCII 透传
        assert_eq!(translate_en("ECOdek 5090"), "ECOdek 5090");
    }

    #[test]
    fn books_render_without_panic() {
        let st = ProjectState::default();
        let out = compute_all(&st, None);
        let h1 = design_book(&st, &out, 3.0);
        let h2 = heat_book(&st, &out, 3.0, "75mm 玻璃丝棉", "75mm 玻璃丝棉");
        assert!(h1.contains("设计计算书") && h1.contains("青岛鑫光正"));
        assert!(h1.contains("图 1") && h1.contains("图 2") && !h1.contains("图 3"));
        assert!(!h1.contains("svg"));
        assert!(h2.contains("热负荷计算书") && h2.contains("Σ(A·K)"));
        assert!(h2.contains("75mm 玻璃丝棉"));
        assert!(h2.contains("保温构造"));
    }

    #[test]
    #[ignore = "手动：生成英文报告冒烟"]
    fn gen_en_current() {
        let state = crate::data::ProjectState::default();
        let out = crate::compute_all(&state, None);
        let p = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("target")
            .join("smoke.json");
        std::fs::write(&p, "{}").ok();
        // units=1 → 触发热源受限，冒烟同时验证英文警告全文
        let f = write_report_en(&state, &out, 1.0, &p.to_string_lossy()).expect("生成失败");
        println!("已生成: {f}");
    }
}

#[cfg(test)]
mod en_probe {
    use super::*;
    #[test]
    #[ignore = "probe"]
    fn en_leftover_probe() {
        let st = crate::data::ProjectState::default();
        let out = crate::compute_all(&st, None);
        let body = post_body(&design_book_body(&st, &out, 2.0, false));
        let en = crate::enlex::translate_en(&body);
        let left = crate::enlex::leftover_cjk(&en);
        println!("LEFTOVER_COUNT={}", left.len());
        for l in &left {
            println!("L|{l}");
        }
    }
}
