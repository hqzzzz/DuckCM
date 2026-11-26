//! 零依赖 OOXML（.docx / Word 2007+ 二进制，zip 容器）生成器：
//! 把本项目自产的 HTML 子集（h1/h2/p/table/tr/td/th/thead/div/b/sup/sub/
//! a 锚点与内链/img base64/br + note/center 类）一步转换为 Word 原生文档。
//! 页眉/页脚/页码域为真 part（任何编辑器不会像 HTML 的 mso-element div 那样
//! 在正文留重复副本）；正文 HTML 原件以孤儿 part `word/source.html` 随包
//! 交付，作为 SM3 验签指纹的计算依据（验签 = 解包重算比对）。

use std::collections::HashMap;

// ═══════════════════════ 公开接口 ═══════════════════════

/// HTML（完整 page() 产物）→ .docx 字节。`date` 写页眉；
/// `source_html` 存进 word/source.html（验签口径原件）。
pub fn build_docx(html: &str, date: &str, source_html: &str) -> Vec<u8> {
    build_docx_themed(html, date, source_html, &DocTheme::cn())
}

/// 页眉/页脚本地化主题（中文册默认；英文册换 en 主题）
pub struct DocTheme {
    pub header_left: String,
    /// 页眉右段前缀（后接日期）
    pub header_right_prefix: String,
    pub footer_left: String,
    /// 页码三段：`第 / 页 / 共 / 页` ↔ `Page / of …`
    pub page_pre: String,
    pub page_mid: String,
    pub page_post: String,
}

impl DocTheme {
    fn cn() -> Self {
        Self {
            header_left: "养殖用房屋设备自动化计算系统".into(),
            header_right_prefix: "生成日期：".into(),
            footer_left: "青岛鑫光正钢结构股份有限公司 · DuckCM 2 · yang.linhao@hotmail.com".into(),
            page_pre: "第 ".into(),
            page_mid: " 页 / 共 ".into(),
            page_post: " 页".into(),
        }
    }
    /// 英文册主题（公司名用官方英文名）
    pub fn en() -> Self {
        Self {
            header_left: "Poultry House Equipment Automated Calculation System".into(),
            header_right_prefix: "Date: ".into(),
            footer_left:
                "Qingdao Xinguangzheng Steel Structure Co., Ltd. · DuckCM 2 · yang.linhao@hotmail.com".into(),
            page_pre: "Page ".into(),
            page_mid: " of ".into(),
            page_post: String::new(),
        }
    }
}

/// 带页眉页脚主题的 docx 生成
pub fn build_docx_themed(html: &str, date: &str, source_html: &str, theme: &DocTheme) -> Vec<u8> {
    let body_nodes = root_children(html);
    let mut cv = Convert::default();
    let body = dedup_pagebreaks(&cv.render_blocks(&body_nodes));
    let document = format!(
        "<?xml version=\"1.0\" encoding=\"UTF-8\" standalone=\"yes\"?>\n\
<w:document {}><w:body>{}\
<w:sectPr><w:headerReference w:type=\"default\" r:id=\"rIdH1\"/><w:footerReference w:type=\"default\" r:id=\"rIdF1\"/>\
<w:pgSz w:w=\"11906\" w:h=\"16838\"/>\
<w:pgMar w:top=\"1134\" w:right=\"1418\" w:bottom=\"1134\" w:left=\"1418\" w:header=\"680\" w:footer=\"680\" w:gutter=\"0\"/>\
</w:sectPr></w:body></w:document>",
        W_NS, body
    );
    let mut rels = String::from(
        "<?xml version=\"1.0\" encoding=\"UTF-8\" standalone=\"yes\"?>\n\
<Relationships xmlns=\"http://schemas.openxmlformats.org/package/2006/relationships\">\
<Relationship Id=\"rIdS\" Type=\"http://schemas.openxmlformats.org/officeDocument/2006/relationships/styles\" Target=\"styles.xml\"/>\
<Relationship Id=\"rIdG\" Type=\"http://schemas.openxmlformats.org/officeDocument/2006/relationships/settings\" Target=\"settings.xml\"/>\
<Relationship Id=\"rIdH1\" Type=\"http://schemas.openxmlformats.org/officeDocument/2006/relationships/header\" Target=\"header1.xml\"/>\
<Relationship Id=\"rIdF1\" Type=\"http://schemas.openxmlformats.org/officeDocument/2006/relationships/footer\" Target=\"footer1.xml\"/>",
    );
    for (i, img) in cv.images.iter().enumerate() {
        rels.push_str(&format!(
            "<Relationship Id=\"rIdI{i}\" Type=\"http://schemas.openxmlformats.org/officeDocument/2006/relationships/image\" Target=\"media/image{i}.png\"/>"
        ));
        let _ = img;
    }
    rels.push_str("</Relationships>");
    let mut files: Vec<ZipFile> = vec![
        ZipFile {
            name: "[Content_Types].xml",
            data: content_types().into_bytes(),
        },
        ZipFile {
            name: "_rels/.rels",
            data: ROOT_RELS.as_bytes().to_vec(),
        },
        ZipFile {
            name: "word/document.xml",
            data: document.into_bytes(),
        },
        ZipFile {
            name: "word/_rels/document.xml.rels",
            data: rels.into_bytes(),
        },
        ZipFile {
            name: "word/styles.xml",
            data: STYLES.as_bytes().to_vec(),
        },
        ZipFile {
            name: "word/settings.xml",
            data: SETTINGS.as_bytes().to_vec(),
        },
        ZipFile {
            name: "word/header1.xml",
            data: header_xml(date, theme).into_bytes(),
        },
        ZipFile {
            name: "word/footer1.xml",
            data: footer_xml(theme).into_bytes(),
        },
        // 验签孤儿 part：Word 未知部件直接忽略，不渲染不报错
        ZipFile {
            name: "word/source.html",
            data: source_html.as_bytes().to_vec(),
        },
    ];
    let img_names: Vec<String> = (0..cv.images.len())
        .map(|i| format!("word/media/image{i}.png"))
        .collect();
    for (name, data) in img_names.iter().zip(cv.images.iter()) {
        files.push(ZipFile {
            name: name.as_str(),
            data: data.clone(),
        });
    }
    zip_store(&files)
}

/// 从 .docx 字节里抽出 word/source.html（验签用）；非 zip / 无该部件返回 None
pub fn extract_source_docx(bytes: &[u8]) -> Option<String> {
    if bytes.len() < 4 || &bytes[0..2] != b"PK" {
        return None;
    }
    let mut pos = 0usize;
    while pos + 30 <= bytes.len() && bytes[pos..pos + 4] == *b"PK\x03\x04" {
        let csize = rd32(&bytes, pos + 18) as usize;
        let nlen = rd16(&bytes, pos + 26) as usize;
        let elen = rd16(&bytes, pos + 28) as usize;
        let name = std::str::from_utf8(&bytes[pos + 30..pos + 30 + nlen]).ok()?;
        let start = pos + 30 + nlen + elen;
        if name == "word/source.html" && start + csize <= bytes.len() {
            return Some(String::from_utf8_lossy(&bytes[start..start + csize]).into_owned());
        }
        pos = start + csize;
    }
    None
}

// ═══════════════════════ HTML → WordML ═══════════════════════

#[derive(Debug, Clone)]
struct Node {
    tag: String,
    attrs: String,
    children: Vec<Node>,
    text: String, // tag 为空串 = 文本节点
}

impl Node {
    fn has(&self, s: &str) -> bool {
        self.attrs.contains(s)
    }
}

const VOID: [&str; 3] = ["br", "img", "meta"];

/// 解析 HTML → 节点树（本项目子集：无畸形标签、无脚本）
fn parse_html(html: &str) -> Vec<Node> {
    let mut roots: Vec<Node> = Vec::new();
    let mut stack: Vec<Node> = Vec::new();
    let b = html.as_bytes();
    let mut i = 0usize;
    let mut text_start = 0usize;
    macro_rules! flush_text {
        ($end:expr) => {
            if $end > text_start {
                let raw = &html[text_start..$end];
                let t = decode_entities(raw);
                if !t.trim().is_empty() || t.contains('\u{a0}') {
                    push(
                        &mut stack,
                        &mut roots,
                        Node {
                            tag: String::new(),
                            attrs: String::new(),
                            children: Vec::new(),
                            text: t,
                        },
                    );
                }
            }
        };
    }
    while i < b.len() {
        if b[i] == b'<' {
            flush_text!(i);
            if html[i..].starts_with("<!--") {
                i = html[i..].find("-->").map(|k| i + k + 3).unwrap_or(b.len());
                text_start = i;
                continue;
            }
            if html[i..].starts_with("<!") || html[i..].starts_with("<?") {
                i = html[i..].find('>').map(|k| i + k + 1).unwrap_or(b.len());
                text_start = i;
                continue;
            }
            let close = html[i..].starts_with("</");
            let end = html[i..].find('>').map(|k| i + k).unwrap_or(b.len());
            let inner = &html[i + if close { 2 } else { 1 }..end];
            let tag: String = inner
                .chars()
                .take_while(|c| c.is_alphanumeric())
                .collect::<String>()
                .to_lowercase();
            if close {
                // 出栈到同名
                while let Some(n) = stack.pop() {
                    let is_match = n.tag == tag;
                    if !n.tag.is_empty() {
                        // n 已在 pop 时携带 children；回挂父级
                        let parent_len = stack.len();
                        attach(&mut stack, &mut roots, n, parent_len);
                    }
                    if is_match {
                        break;
                    }
                }
            } else {
                let attrs = inner[tag.len()..].to_string();
                let node = Node {
                    tag: tag.clone(),
                    attrs,
                    children: Vec::new(),
                    text: String::new(),
                };
                if VOID.contains(&tag.as_str()) {
                    push(&mut stack, &mut roots, node);
                } else {
                    stack.push(node);
                }
            }
            i = end + 1;
            text_start = i;
        } else {
            i += 1;
        }
    }
    flush_text!(b.len());
    while let Some(n) = stack.pop() {
        let parent_len = stack.len();
        attach(&mut stack, &mut roots, n, parent_len);
    }
    roots
}

fn push(stack: &mut Vec<Node>, roots: &mut Vec<Node>, n: Node) {
    if let Some(parent) = stack.last_mut() {
        parent.children.push(n);
    } else {
        roots.push(n);
    }
}

/// pop 出的节点回挂：其父 = 当前栈顶（stack 在 pop 后长度即 parent_len）
fn attach(stack: &mut Vec<Node>, roots: &mut Vec<Node>, n: Node, _parent_len: usize) {
    push(stack, roots, n);
}

/// 取 body → div.WordSection1 的子节点
fn root_children(html: &str) -> Vec<Node> {
    fn find_section(nodes: &[Node]) -> Option<Vec<Node>> {
        for n in nodes {
            if n.tag == "div" && n.has("WordSection1") {
                return Some(n.children.clone());
            }
            if let Some(v) = find_section(&n.children) {
                return Some(v);
            }
        }
        None
    }
    let roots = parse_html(html);
    find_section(&roots).unwrap_or(roots)
}

#[derive(Default)]
struct Convert {
    images: Vec<Vec<u8>>,
    bm_id: usize,
    bookmarks: HashMap<String, usize>,
    /// 上一 div 要求 page-break-after：给下一个块加 w:pageBreakBefore
    /// 段属性（不用独立分页空段——空段自身跨页会在末页顶出整页空白）
    brk_next: bool,
}

impl Convert {
    /// 块级渲染
    fn render_blocks(&mut self, nodes: &[Node]) -> String {
        let mut out = String::new();
        for n in nodes {
            match n.tag.as_str() {
                "h1" | "h2" | "h3" => {
                    // 标题带 page-break-before（如「结论单起一页」）→ 段属性分页
                    let self_brk = n.style_has("page-break-before");
                    let pb = if self_brk || self.brk_next {
                        self.brk_next = false;
                        "<w:pageBreakBefore/>"
                    } else {
                        ""
                    };
                    let (style, sz) = match n.tag.as_str() {
                        "h1" => ("Heading1", 36),
                        "h2" => ("Heading2", 28),
                        _ => ("Heading3", 24),
                    };
                    out.push_str(&format!(
                        "<w:p><w:pPr>{pb}<w:pStyle w:val=\"{style}\"/></w:pPr>"
                    ));
                    out.push_str(&self.inlines(&n.children, &Rt::head(sz)));
                    out.push_str("</w:p>");
                }
                "p" => out.push_str(&self.paragraph(n)),
                "table" => {
                    if self.brk_next {
                        out.push_str(PAGEBREAK_P);
                        self.brk_next = false;
                    }
                    out.push_str(&self.table(n, true))
                }
                "div" => {
                    // 封面：page-break-after → 下一个顶层块段前分页（属性方式）
                    let mut inner = String::new();
                    std::mem::swap(&mut inner, &mut out);
                    let blocks = self.render_blocks(&n.children);
                    out = inner;
                    out.push_str(&blocks);
                    if n.has("page-break-after:always") {
                        self.brk_next = true;
                    }
                }
                "" => {
                    // 游离文本 → 普通段落（带首行缩进）
                    let t = self.inlines(&[n.clone()], &Rt::body());
                    if !t.is_empty() {
                        out.push_str(&format!(
                            "<w:p><w:pPr><w:ind w:firstLine=\"480\"/></w:pPr>{t}</w:p>"
                        ));
                    }
                }
                _ => {
                    let blocks = self.render_blocks(&n.children);
                    out.push_str(&blocks);
                }
            }
        }
        out
    }

    /// 段落：p 的 class/style 映射（note/center/MsoHeader/Consolas/
    /// page-break-before/首行缩进）；含图片的段 keepNext（与图名不分页）
    fn paragraph(&mut self, n: &Node) -> String {
        let mut ppr = String::from("<w:pPr>");
        let mut rt = Rt::body();
        // 段前分页：w:pageBreakBefore 属性（含上一 div 的分页要求转嫁给本段）
        if n.has("page-break-before") || self.brk_next {
            ppr.push_str("<w:pageBreakBefore/>");
            self.brk_next = false;
        }
        if subtree_has_img(n) {
            ppr.push_str("<w:keepNext/>");
        }
        let note = n.has("note");
        let center = n.contains_class("center") || n.style_has("text-align:center");
        if note {
            rt.color = Some("555555");
            rt.sz = 20;
        }
        if n.style_has("Consolas") {
            rt.mono = true;
            rt.sz = 19;
        }
        if center {
            ppr.push_str("<w:jc w:val=\"center\"/>");
        }
        // margin-top:110pt（封面 h1 在 div 内，这里 p 也可能带）
        if let Some(pt) = style_num(n.attrs.as_str(), "margin-top:") {
            ppr.push_str(&format!("<w:spacing w:before=\"{}\"/>", pt * 20));
        }
        if !note && !center {
            ppr.push_str("<w:ind w:firstLine=\"480\"/>");
        }
        if !n.has("note") && !center && !n.has("Consolas") {
            ppr.push_str("<w:spacing w:after=\"80\"/>");
        }
        ppr.push_str("</w:pPr>");
        format!("<w:p>{ppr}{}</w:p>", self.inlines(&n.children, &rt))
    }

    /// 表格：全边框 + th 底纹加粗 + thead 行跨页重复 + align=center 居中；
    /// class=charttbl（条形图）整表 + 下方图名不分页（全行 keepNext + cantSplit）；
    /// style 含 border:none 则无框线（条形图外框/色条轨道）；bgcolor/width/
    /// height 属性支持（色条）；嵌套表格（色条内表）在 inlines 递归渲染
    fn table(&mut self, n: &Node, spacer: bool) -> String {
        let chart = n.has("charttbl");
        let center = n.has("align=\"center\"") || n.style_has("margin-left:auto");
        let noborder = n.style_has("border:none");
        let borders = if noborder {
            "<w:tblBorders>\
<w:top w:val=\"none\"/><w:left w:val=\"none\"/><w:bottom w:val=\"none\"/>\
<w:right w:val=\"none\"/><w:insideH w:val=\"none\"/><w:insideV w:val=\"none\"/>\
</w:tblBorders><w:tblCellMar><w:left w:w=\"0\" w:type=\"dxa\"/><w:right w:w=\"0\" w:type=\"dxa\"/>\
<w:top w:w=\"0\" w:type=\"dxa\"/><w:bottom w:w=\"0\" w:type=\"dxa\"/></w:tblCellMar>"
        } else {
            "<w:tblBorders>\
<w:top w:val=\"single\" w:sz=\"6\" w:color=\"444444\"/><w:left w:val=\"single\" w:sz=\"6\" w:color=\"444444\"/>\
<w:bottom w:val=\"single\" w:sz=\"6\" w:color=\"444444\"/><w:right w:val=\"single\" w:sz=\"6\" w:color=\"444444\"/>\
<w:insideH w:val=\"single\" w:sz=\"6\" w:color=\"444444\"/><w:insideV w:val=\"single\" w:sz=\"6\" w:color=\"444444\"/>\
</w:tblBorders><w:tblCellMar><w:left w:w=\"113\" w:type=\"dxa\"/><w:right w:w=\"113\" w:type=\"dxa\"/>\
<w:top w:w=\"28\" w:type=\"dxa\"/><w:bottom w:w=\"28\" w:type=\"dxa\"/></w:tblCellMar>"
        };
        // 外框统一 100% 版心宽（各表等宽；列宽仍按内容自动分配）
        let mut tblpr = format!("<w:tblPr><w:tblW w:w=\"5000\" w:type=\"pct\"/>{borders}");
        if center {
            tblpr.push_str("<w:jc w:val=\"center\"/>");
        }
        tblpr.push_str("</w:tblPr>");
        let mut out = format!("<w:tbl>{tblpr}<w:tblGrid/>");
        for tr in n
            .children
            .iter()
            .filter(|c| c.tag == "tr" || c.tag == "thead")
        {
            let header_row = tr.tag == "thead";
            let trs: Vec<&Node> = if header_row {
                tr.children.iter().filter(|c| c.tag == "tr").collect()
            } else {
                vec![tr]
            };
            for tr in trs {
                out.push_str("<w:tr>");
                if header_row {
                    out.push_str("<w:trPr><w:tblHeader/></w:trPr>");
                }
                // 行高：任一单元格 height=N（px）→ 240twips/12px 比例 20×px
                let hpx = tr
                    .children
                    .iter()
                    .filter(|c| c.tag == "td" || c.tag == "th")
                    .filter_map(|c| attr_num(&c.attrs, "height"))
                    .max();
                let cant = if chart { "<w:cantSplit/>" } else { "" };
                let hgt =
                    hpx.map(|p| format!("<w:trHeight w:val=\"{}\" w:hRule=\"atLeast\"/>", p * 20));
                let trpr = format!("{cant}{}", hgt.unwrap_or_default());
                if !trpr.is_empty() {
                    out.push_str(&format!("<w:trPr>{trpr}</w:trPr>"));
                }
                for cell in tr
                    .children
                    .iter()
                    .filter(|c| c.tag == "td" || c.tag == "th")
                {
                    let th = cell.tag == "th";
                    let mut tcpr = String::from("<w:tcPr>");
                    // 宽度：width="38.5%" → pct（×50）；width="150" → dxa（×15px→twip）
                    if let Some(w) = attr_val(&cell.attrs, "width") {
                        if let Some(pct) = w.strip_suffix('%') {
                            if let Ok(p) = pct.trim().parse::<f64>() {
                                tcpr.push_str(&format!(
                                    "<w:tcW w:w=\"{}\" w:type=\"pct\"/>",
                                    (p * 50.0).round() as i64
                                ));
                            }
                        } else if let Ok(px) = w.trim().parse::<i64>() {
                            tcpr.push_str(&format!("<w:tcW w:w=\"{}\" w:type=\"dxa\"/>", px * 15));
                        }
                    }
                    if th {
                        tcpr.push_str("<w:shd w:val=\"clear\" w:fill=\"EEF3F8\"/>");
                    }
                    // bgcolor 属性 / style background → 底纹（色条）
                    let bg = attr_val(&cell.attrs, "bgcolor")
                        .map(|v| v.trim_start_matches('#').to_uppercase())
                        .filter(|v| v.len() == 6 && v.chars().all(|c| c.is_ascii_hexdigit()))
                        .or_else(|| bg_from_style(&cell.attrs));
                    if let Some(c) = bg {
                        tcpr.push_str(&format!("<w:shd w:val=\"clear\" w:fill=\"{c}\"/>"));
                    }
                    tcpr.push_str("</w:tcPr>");
                    let mut rt = Rt::cell();
                    if th {
                        rt.bold = true;
                    }
                    let keep = if chart { "<w:keepNext/>" } else { "" };
                    // 单元格内容：段落 runs；若含嵌套表（色条轨道）则表 + 尾随空段
                    let has_tbl = cell.children.iter().any(|c| c.tag == "table");
                    if has_tbl {
                        let mut inner = String::new();
                        for c in &cell.children {
                            if c.tag == "table" {
                                inner.push_str(&self.table(c, false));
                            } else {
                                let r = self.inlines(std::slice::from_ref(c), &rt);
                                if !r.is_empty() {
                                    inner.push_str(&format!(
                                        "<w:p><w:pPr><w:spacing w:after=\"0\" w:line=\"240\" w:lineRule=\"auto\"/><w:ind w:firstLine=\"0\"/></w:pPr>{r}</w:p>"
                                    ));
                                }
                            }
                        }
                        // OOXML：w:tc 最后子元素必须是 w:p（嵌套表收尾空段）
                        let keep2 = if chart { "<w:keepNext/>" } else { "" };
                        if inner.ends_with("</w:tbl>") {
                            inner.push_str(&format!(
                                "<w:p><w:pPr>{keep2}<w:spacing w:after=\"0\" w:line=\"240\" w:lineRule=\"auto\"/></w:pPr></w:p>"
                            ));
                        }
                        out.push_str(&format!("<w:tc>{tcpr}{inner}</w:tc>"));
                    } else {
                        let runs = self.inlines(&cell.children, &rt);
                        out.push_str(&format!(
                            "<w:tc>{tcpr}<w:p><w:pPr>{keep}<w:spacing w:after=\"0\" w:line=\"240\" w:lineRule=\"auto\"/><w:ind w:firstLine=\"0\"/></w:pPr>{runs}</w:p></w:tc>"
                        ));
                    }
                }
                out.push_str("</w:tr>");
            }
        }
        out.push_str("</w:tbl>");
        if spacer {
            // 表后空段（防表格粘连下文的呼吸位）；charttbl 时该段也 keepNext，
            // 使 表→空段→图名 连成不分页整体
            if chart {
                out.push_str("<w:p><w:pPr><w:keepNext/><w:spacing w:after=\"0\" w:line=\"240\" w:lineRule=\"auto\"/></w:pPr></w:p>");
            } else {
                out.push_str("<w:p><w:pPr><w:spacing w:after=\"0\" w:line=\"240\" w:lineRule=\"auto\"/></w:pPr></w:p>");
            }
        }
        out
    }

    /// 行内内容 → runs；ctx 携带 b/sup/sub/color 继承
    fn inlines(&mut self, nodes: &[Node], rt: &Rt) -> String {
        let mut out = String::new();
        for n in nodes {
            match n.tag.as_str() {
                "" => out.push_str(&run(&xml_escape(&tail_trim(&n.text)), rt)),
                "b" => {
                    let mut r2 = rt.clone();
                    r2.bold = true;
                    out.push_str(&self.inlines(&n.children, &r2));
                }
                "sup" => {
                    let mut r2 = rt.clone();
                    r2.sup = true;
                    out.push_str(&self.inlines(&n.children, &r2));
                }
                "sub" => {
                    let mut r2 = rt.clone();
                    r2.sub = true;
                    out.push_str(&self.inlines(&n.children, &r2));
                }
                "br" => out.push_str(&format!("{}<w:br/>", run("", rt))),
                // span/font 带 background → 字符底纹 run（条形图色条）
                "span" | "font" => {
                    let mut r2 = rt.clone();
                    if let Some(hex) = bg_from_style(&n.attrs) {
                        r2.bg = Some(hex);
                    }
                    if let Some(hex) = attr_val(&n.attrs, "color")
                        .map(|c| c.trim_start_matches('#').to_uppercase())
                    {
                        if hex.len() == 6 && hex.chars().all(|ch| ch.is_ascii_hexdigit()) {
                            r2.color2 = Some(hex);
                        }
                    }
                    out.push_str(&self.inlines(&n.children, &r2));
                }
                "a" => {
                    if let Some(name) = attr_val(&n.attrs, "name") {
                        // 书签锚点（附录公式序号）：包裹子内容，文本照常输出
                        let id = self.bm_id;
                        self.bm_id += 1;
                        self.bookmarks.insert(name.clone(), id);
                        let inner = self.inlines(&n.children, rt);
                        out.push_str(&format!(
                            "<w:bookmarkStart w:id=\"{id}\" w:name=\"{name}\"/>{inner}<w:bookmarkEnd w:id=\"{id}\"/>"
                        ));
                    } else if let Some(href) = attr_val(&n.attrs, "href") {
                        if let Some(anchor) = href.strip_prefix('#') {
                            let mut r2 = rt.clone();
                            r2.color = Some("2E5FA3");
                            r2.underline = true;
                            let inner = self.inlines(&n.children, &r2);
                            out.push_str(&format!(
                                "<w:hyperlink w:anchor=\"{anchor}\">{inner}</w:hyperlink>"
                            ));
                        }
                    }
                }
                "img" => {
                    if let Some(src) = attr_val(&n.attrs, "src") {
                        if let Some(b64) = src.strip_prefix("data:image/png;base64,") {
                            if let Some(bytes) = b64_decode(b64) {
                                let idx = self.images.len();
                                self.images.push(bytes);
                                let w = attr_num(&n.attrs, "width").unwrap_or(424) as i64;
                                let h = attr_num(&n.attrs, "height").unwrap_or(60) as i64;
                                out.push_str(&drawing(idx, w, h));
                            }
                        }
                    }
                }
                _ => out.push_str(&self.inlines(&n.children, rt)),
            }
        }
        out
    }
}

trait HasClass {
    fn contains_class(&self, c: &str) -> bool;
    fn style_has(&self, s: &str) -> bool;
}
impl HasClass for Node {
    fn contains_class(&self, c: &str) -> bool {
        attr_val(&self.attrs, "class").map(|v| v.split_whitespace().any(|x| x == c)) == Some(true)
    }
    fn style_has(&self, s: &str) -> bool {
        self.attrs.replace(' ', "").contains(s)
    }
}

/// 行内样式上下文
#[derive(Clone)]
struct Rt {
    bold: bool,
    sup: bool,
    sub: bool,
    color: Option<&'static str>,
    /// 动态前景色（HTML color 属性，来自 font/span）
    color2: Option<String>,
    /// 字符底纹（条形图色条）
    bg: Option<String>,
    mono: bool,
    underline: bool,
    sz: u32, // half-points
}
impl Rt {
    fn body() -> Self {
        Self {
            bold: false,
            sup: false,
            sub: false,
            color: None,
            color2: None,
            bg: None,
            mono: false,
            underline: false,
            sz: 24,
        }
    }
    fn head(sz: u32) -> Self {
        Self {
            bold: true,
            sz,
            ..Self::body()
        }
    }
    fn cell() -> Self {
        Self {
            sz: 21,
            ..Self::body()
        }
    }
    fn rpr(&self) -> String {
        let mut s = String::from("<w:rPr>");
        if self.mono {
            s.push_str("<w:rFonts w:ascii=\"Consolas\" w:hAnsi=\"Consolas\" w:eastAsia=\"宋体\"/>");
        } else {
            s.push_str("<w:rFonts w:ascii=\"Times New Roman\" w:hAnsi=\"Times New Roman\" w:eastAsia=\"宋体\"/>");
        }
        if self.bold {
            s.push_str("<w:b/><w:bCs/>");
        }
        if self.underline {
            s.push_str("<w:u w:val=\"single\"/>");
        }
        if let Some(c) = self.color {
            s.push_str(&format!("<w:color w:val=\"{c}\"/>"));
        } else if let Some(c) = self.color2.as_deref() {
            s.push_str(&format!("<w:color w:val=\"{c}\"/>"));
        }
        if let Some(bg) = self.bg.as_deref() {
            s.push_str(&format!("<w:shd w:val=\"clear\" w:fill=\"{bg}\"/>"));
        }
        if self.sup {
            s.push_str("<w:vertAlign w:val=\"superscript\"/>");
        }
        if self.sub {
            s.push_str("<w:vertAlign w:val=\"subscript\"/>");
        }
        s.push_str(&format!(
            "<w:sz w:val=\"{}\"/><w:szCs w:val=\"{}\"/></w:rPr>",
            self.sz, self.sz
        ));
        s
    }
}

fn run(text: &str, rt: &Rt) -> String {
    if text.is_empty() {
        return String::new();
    }
    format!(
        "<w:r>{}<w:t xml:space=\"preserve\">{}</w:t></w:r>",
        rt.rpr(),
        text
    )
}

/// 子树是否含 <img>（图片段 keepNext 用）
fn subtree_has_img(n: &Node) -> bool {
    n.tag == "img" || n.children.iter().any(subtree_has_img)
}

/// style 属性里的 background[:color] → 大写 hex（去 #）
fn bg_from_style(attrs: &str) -> Option<String> {
    let a = attrs.replace(' ', "").to_lowercase();
    let i = a.find("background:")? + "background:".len();
    let rest = &a[i..];
    let end = rest.find([';', '"', '\'']).unwrap_or(rest.len());
    let v = rest[..end].trim_start_matches('#');
    if v.len() == 6 && v.chars().all(|c| c.is_ascii_hexdigit()) {
        Some(v.to_uppercase())
    } else {
        None
    }
}

/// 尾部纯空白交给 Word 排版，nbsp 保留（占位段）
fn tail_trim(t: &str) -> String {
    t.to_string()
}

// ═══════════════════════ 工具 ═══════════════════════

fn decode_entities(s: &str) -> String {
    let s = s
        .replace("&nbsp;", "\u{a0}")
        .replace("&amp;", "&")
        .replace("&lt;", "<")
        .replace("&gt;", ">")
        .replace("&middot;", "·");
    decode_numeric_entities(&s)
}

/// 还原数字字符实体 `&#931;` / `&#x3A3;` → 对应 Unicode 字符
/// （英文报告用实体书写 Σ · α ε Δ ℃ × ≤ — 等符号）
fn decode_numeric_entities(s: &str) -> String {
    if !s.contains("&#") {
        return s.to_string();
    }
    let b = s.as_bytes();
    let mut out = String::with_capacity(s.len());
    let mut i = 0;
    while i < b.len() {
        if b[i] == b'&' && i + 2 < b.len() && b[i + 1] == b'#' {
            let hex = b[i + 2] == b'x' || b[i + 2] == b'X';
            let start = i + if hex { 3 } else { 2 };
            let mut j = start;
            while j < b.len() && (b[j] as char).is_ascii_alphanumeric() {
                j += 1;
            }
            if j < b.len() && b[j] == b';' && j > start {
                if let Some(n) = u32::from_str_radix(&s[start..j], if hex { 16 } else { 10 }).ok() {
                    if let Some(c) = char::from_u32(n) {
                        out.push(c);
                        i = j + 1;
                        continue;
                    }
                }
            }
        }
        out.push(s[i..].chars().next().unwrap());
        i += s[i..].chars().next().unwrap().len_utf8();
    }
    out
}

fn xml_escape(s: &str) -> String {
    s.replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
}

fn attr_val<'a>(attrs: &'a str, name: &str) -> Option<String> {
    let lower = attrs.to_lowercase();
    let key = name.to_lowercase();
    let i = lower.find(&key)?;
    let rest = &attrs[i + key.len()..].trim_start();
    let rest = rest.strip_prefix('=')?.trim_start();
    if let Some(r) = rest.strip_prefix('"') {
        r.find('"').map(|k| r[..k].to_string())
    } else {
        let end = rest.find([' ', '\t', '\n', '>']).unwrap_or(rest.len());
        Some(rest[..end].to_string())
    }
}

fn attr_num(attrs: &str, name: &str) -> Option<u32> {
    attr_val(attrs, name).and_then(|v| v.trim_end_matches("px").trim().parse::<u32>().ok())
}

/// 从 attrs 中提取 "prefix:数字(pt)" 的数字
fn style_num(attrs: &str, prefix: &str) -> Option<i64> {
    let a = attrs.replace(' ', "");
    let i = a.find(prefix)?;
    let rest = &a[i + prefix.len()..];
    let end = rest
        .find(|c: char| !(c.is_ascii_digit() || c == '.'))
        .unwrap_or(rest.len());
    rest[..end].parse::<f64>().ok().map(|v| v as i64)
}

fn b64_decode(s: &str) -> Option<Vec<u8>> {
    fn val(c: u8) -> Option<u8> {
        match c {
            b'A'..=b'Z' => Some(c - b'A'),
            b'a'..=b'z' => Some(c - b'a' + 26),
            b'0'..=b'9' => Some(c - b'0' + 52),
            b'+' => Some(62),
            b'/' => Some(63),
            _ => None,
        }
    }
    let bytes: Vec<u8> = s
        .bytes()
        .filter(|b| !b.is_ascii_whitespace() && *b != b'=')
        .collect();
    let mut out = Vec::with_capacity(bytes.len() * 3 / 4);
    let mut acc: u32 = 0;
    let mut nbits = 0u32;
    for c in &bytes {
        acc = (acc << 6) | val(*c)? as u32;
        nbits += 6;
        if nbits >= 8 {
            nbits -= 8;
            out.push((acc >> nbits) as u8);
        }
    }
    Some(out)
}

/// inline drawing：px→EMU（×9525）
fn drawing(idx: usize, w_px: i64, h_px: i64) -> String {
    let cx = w_px * 9525;
    let cy = h_px * 9525;
    format!(
        "<w:r><w:drawing><wp:inline distT=\"0\" distB=\"0\" distL=\"0\" distR=\"0\">\
<wp:extent cx=\"{cx}\" cy=\"{cy}\"/><wp:docPr id=\"{}\" name=\"img{idx}\"/>\
<a:graphic xmlns:a=\"http://schemas.openxmlformats.org/drawingml/2006/main\">\
<a:graphicData uri=\"http://schemas.openxmlformats.org/drawingml/2006/picture\">\
<pic:pic xmlns:pic=\"http://schemas.openxmlformats.org/drawingml/2006/picture\">\
<pic:nvPicPr><pic:cNvPr id=\"{}\" name=\"image{idx}.png\"/><pic:cNvPicPr/></pic:nvPicPr>\
<pic:blipFill><a:blip r:embed=\"rIdI{idx}\"/><a:stretch><a:fillRect/></a:stretch></pic:blipFill>\
<pic:spPr><a:xfrm><a:off x=\"0\" y=\"0\"/><a:ext cx=\"{cx}\" cy=\"{cy}\"/></a:xfrm>\
<a:prstGeom prst=\"rect\"><a:avLst/></a:prstGeom></pic:spPr></pic:pic></a:graphicData></a:graphic>\
</wp:inline></w:drawing></w:r>",
        100 + idx,
        100 + idx
    )
}

const PAGEBREAK_P: &str = "<w:p><w:r><w:br w:type=\"page\"/></w:r></w:p>";

/// 相邻/间隔空段的重复分页符归一（封面 page-break-after + 章节
/// page-break-before 相邻会出空白页）
fn dedup_pagebreaks(body: &str) -> String {
    let mut s = body.to_string();
    loop {
        let before = s.clone();
        // 直接相邻
        s = s.replace(&format!("{P}{P}", P = PAGEBREAK_P), PAGEBREAK_P);
        // 中间夹空段（spacer / 空 &nbsp; 段）
        let empty1 = "<w:p><w:pPr><w:spacing w:after=\"0\" w:line=\"240\" w:lineRule=\"auto\"/></w:pPr></w:p>";
        let empty2 = "<w:p><w:pPr><w:spacing w:after=\"80\"/><w:ind w:firstLine=\"480\"/></w:pPr><w:r><w:rPr><w:rFonts w:ascii=\"Times New Roman\" w:hAnsi=\"Times New Roman\" w:eastAsia=\"宋体\"/><w:sz w:val=\"24\"/><w:szCs w:val=\"24\"/></w:rPr><w:t xml:space=\"preserve\">\u{a0}</w:t></w:r></w:p>";
        for mid in [empty1, empty2] {
            s = s.replace(&format!("{P}{M}{P}", P = PAGEBREAK_P, M = mid), PAGEBREAK_P);
            let _ = empty2;
        }
        if s == before {
            return s;
        }
    }
}

// ═══════════════════════ 固定部件 ═══════════════════════

const W_NS: &str = "xmlns:w=\"http://schemas.openxmlformats.org/wordprocessingml/2006/main\" \
xmlns:r=\"http://schemas.openxmlformats.org/officeDocument/2006/relationships\" \
xmlns:wp=\"http://schemas.openxmlformats.org/drawingml/2006/wordprocessingDrawing\"";

const ROOT_RELS: &str = "<?xml version=\"1.0\" encoding=\"UTF-8\" standalone=\"yes\"?>\n\
<Relationships xmlns=\"http://schemas.openxmlformats.org/package/2006/relationships\">\
<Relationship Id=\"rId1\" Type=\"http://schemas.openxmlformats.org/officeDocument/2006/relationships/officeDocument\" Target=\"word/document.xml\"/>\
</Relationships>";

fn content_types() -> String {
    String::from(
        "<?xml version=\"1.0\" encoding=\"UTF-8\" standalone=\"yes\"?>\n\
<Types xmlns=\"http://schemas.openxmlformats.org/package/2006/content-types\">\
<Default Extension=\"rels\" ContentType=\"application/vnd.openxmlformats-package.relationships+xml\"/>\
<Default Extension=\"xml\" ContentType=\"application/xml\"/>\
<Default Extension=\"png\" ContentType=\"image/png\"/>\
<Default Extension=\"html\" ContentType=\"text/html\"/>\
<Override PartName=\"/word/document.xml\" ContentType=\"application/vnd.openxmlformats-officedocument.wordprocessingml.document.main+xml\"/>\
<Override PartName=\"/word/styles.xml\" ContentType=\"application/vnd.openxmlformats-officedocument.wordprocessingml.styles+xml\"/>\
<Override PartName=\"/word/settings.xml\" ContentType=\"application/vnd.openxmlformats-officedocument.wordprocessingml.settings+xml\"/>\
<Override PartName=\"/word/header1.xml\" ContentType=\"application/vnd.openxmlformats-officedocument.wordprocessingml.header+xml\"/>\
<Override PartName=\"/word/footer1.xml\" ContentType=\"application/vnd.openxmlformats-officedocument.wordprocessingml.footer+xml\"/>\
</Types>",
    )
}

const STYLES: &str = "<?xml version=\"1.0\" encoding=\"UTF-8\" standalone=\"yes\"?>\n\
<w:styles xmlns:w=\"http://schemas.openxmlformats.org/wordprocessingml/2006/main\">\
<w:docDefaults><w:rPrDefault><w:rPr>\
<w:rFonts w:ascii=\"Times New Roman\" w:hAnsi=\"Times New Roman\" w:eastAsia=\"宋体\" w:cs=\"Times New Roman\"/>\
<w:sz w:val=\"24\"/><w:szCs w:val=\"24\"/></w:rPr></w:rPrDefault>\
<w:pPrDefault><w:pPr><w:spacing w:after=\"80\" w:line=\"360\" w:lineRule=\"auto\"/></w:pPr></w:pPrDefault></w:docDefaults>\
<w:style w:type=\"paragraph\" w:default=\"1\" w:styleId=\"Normal\"><w:name w:val=\"Normal\"/></w:style>\
<w:style w:type=\"paragraph\" w:styleId=\"Heading1\"><w:name w:val=\"heading 1\"/>\
<w:pPr><w:keepNext/><w:jc w:val=\"center\"/><w:spacing w:before=\"120\" w:after=\"120\"/></w:pPr>\
<w:rPr><w:b/><w:bCs/><w:sz w:val=\"36\"/><w:szCs w:val=\"36\"/></w:rPr></w:style>\
<w:style w:type=\"paragraph\" w:styleId=\"Heading2\"><w:name w:val=\"heading 2\"/>\
<w:pPr><w:keepNext/><w:spacing w:before=\"280\" w:after=\"80\"/></w:pPr>\
<w:rPr><w:b/><w:bCs/><w:sz w:val=\"28\"/><w:szCs w:val=\"28\"/></w:rPr></w:style>\
<w:style w:type=\"paragraph\" w:styleId=\"Heading3\"><w:name w:val=\"heading 3\"/>\
<w:pPr><w:keepNext/><w:spacing w:before=\"200\" w:after=\"60\"/></w:pPr>\
<w:rPr><w:b/><w:bCs/><w:sz w:val=\"24\"/><w:szCs w:val=\"24\"/></w:rPr></w:style>\
</w:styles>";

const SETTINGS: &str = "<?xml version=\"1.0\" encoding=\"UTF-8\" standalone=\"yes\"?>\n\
<w:settings xmlns:w=\"http://schemas.openxmlformats.org/wordprocessingml/2006/main\">\
<w:defaultTabStop w:val=\"420\"/></w:settings>";

fn header_xml(date: &str, theme: &DocTheme) -> String {
    let rf = "<w:rPr><w:rFonts w:ascii=\"Times New Roman\" w:hAnsi=\"Times New Roman\" w:eastAsia=\"宋体\"/>\
<w:color w:val=\"555555\"/><w:sz w:val=\"18\"/><w:szCs w:val=\"18\"/></w:rPr>";
    format!(
        "<?xml version=\"1.0\" encoding=\"UTF-8\" standalone=\"yes\"?>\n\
<w:hdr {W_NS}><w:p><w:pPr><w:pBdr><w:bottom w:val=\"single\" w:sz=\"4\" w:space=\"1\" w:color=\"888888\"/></w:pBdr>\
<w:tabs><w:tab w:val=\"right\" w:pos=\"9070\"/></w:tabs><w:spacing w:after=\"0\" w:line=\"240\" w:lineRule=\"auto\"/>\
<w:jc w:val=\"left\"/></w:pPr>\
<w:r>{rf}<w:t xml:space=\"preserve\">{}</w:t></w:r>\
<w:r>{rf}<w:tab/></w:r>\
<w:r>{rf}<w:t xml:space=\"preserve\">{}{}</w:t></w:r></w:p></w:hdr>",
        xml_escape(&theme.header_left),
        xml_escape(&theme.header_right_prefix),
        date
    )
}

fn footer_xml(theme: &DocTheme) -> String {
    let rf = "<w:rPr><w:rFonts w:ascii=\"Times New Roman\" w:hAnsi=\"Times New Roman\" w:eastAsia=\"宋体\"/>\
<w:color w:val=\"555555\"/><w:sz w:val=\"18\"/><w:szCs w:val=\"18\"/></w:rPr>";
    let post = &theme.page_post;
    let post_run = if post.is_empty() {
        String::new()
    } else {
        format!("<w:r>{rf}<w:t xml:space=\"preserve\">{post}</w:t></w:r>")
    };
    format!(
        "<?xml version=\"1.0\" encoding=\"UTF-8\" standalone=\"yes\"?>\n\
<w:ftr {W_NS}><w:p><w:pPr><w:pBdr><w:top w:val=\"single\" w:sz=\"4\" w:space=\"1\" w:color=\"888888\"/></w:pBdr>\
<w:tabs><w:tab w:val=\"right\" w:pos=\"9070\"/></w:tabs>\
<w:spacing w:after=\"0\" w:line=\"240\" w:lineRule=\"auto\"/><w:jc w:val=\"left\"/></w:pPr>\
<w:r>{rf}<w:t xml:space=\"preserve\">{}</w:t></w:r>\
<w:r>{rf}<w:tab/></w:r>\
<w:r>{rf}<w:t xml:space=\"preserve\">{}</w:t></w:r>\
<w:fldSimple w:instr=\" PAGE \\\\* MERGEFORMAT \"><w:r>{rf}<w:t xml:space=\"preserve\">1</w:t></w:r></w:fldSimple>\
<w:r>{rf}<w:t>{}</w:t></w:r>\
<w:fldSimple w:instr=\" NUMPAGES \\\\* MERGEFORMAT \"><w:r>{rf}<w:t xml:space=\"preserve\">1</w:t></w:r></w:fldSimple>\
{post_run}</w:p></w:ftr>",
        xml_escape(&theme.footer_left),
        xml_escape(&theme.page_pre),
        xml_escape(&theme.page_mid)
    )
}

// ═══════════════════════ zip（STORE 无压缩）═══════════════════════

struct ZipFile<'a> {
    name: &'a str,
    data: Vec<u8>,
}

fn crc32(bytes: &[u8]) -> u32 {
    let mut crc = !0u32;
    for b in bytes {
        crc ^= *b as u32;
        for _ in 0..8 {
            let mask = (crc & 1).wrapping_neg();
            crc = (crc >> 1) ^ (0xEDB8_8320 & mask);
        }
    }
    !crc
}

fn rd16(b: &[u8], i: usize) -> u16 {
    u16::from_le_bytes([b[i], b[i + 1]])
}
fn rd32(b: &[u8], i: usize) -> u32 {
    u32::from_le_bytes([b[i], b[i + 1], b[i + 2], b[i + 3]])
}

fn zip_store(files: &[ZipFile]) -> Vec<u8> {
    let dos_date: u16 = ((2026 - 1980) << 9) | (1 << 5) | 1;
    let dos_time: u16 = 0;
    let mut out: Vec<u8> = Vec::new();
    let mut central: Vec<u8> = Vec::new();
    let mut count: u16 = 0;
    for f in files {
        let crc = crc32(&f.data);
        let nlen = f.name.len() as u16;
        let size = f.data.len() as u32;
        let offset = out.len() as u32;
        // local file header
        out.extend_from_slice(&[0x50, 0x4b, 0x03, 0x04]);
        out.extend_from_slice(&20u16.to_le_bytes()); // version
        out.extend_from_slice(&0u16.to_le_bytes()); // flags
        out.extend_from_slice(&0u16.to_le_bytes()); // method STORE
        out.extend_from_slice(&dos_time.to_le_bytes());
        out.extend_from_slice(&dos_date.to_le_bytes());
        out.extend_from_slice(&crc.to_le_bytes());
        out.extend_from_slice(&size.to_le_bytes());
        out.extend_from_slice(&size.to_le_bytes());
        out.extend_from_slice(&nlen.to_le_bytes());
        out.extend_from_slice(&0u16.to_le_bytes()); // extra len
        out.extend_from_slice(f.name.as_bytes());
        out.extend_from_slice(&f.data);
        // central directory
        central.extend_from_slice(&[0x50, 0x4b, 0x01, 0x02]);
        central.extend_from_slice(&20u16.to_le_bytes()); // made by
        central.extend_from_slice(&20u16.to_le_bytes()); // need
        central.extend_from_slice(&0u16.to_le_bytes());
        central.extend_from_slice(&0u16.to_le_bytes());
        central.extend_from_slice(&dos_time.to_le_bytes());
        central.extend_from_slice(&dos_date.to_le_bytes());
        central.extend_from_slice(&crc.to_le_bytes());
        central.extend_from_slice(&size.to_le_bytes());
        central.extend_from_slice(&size.to_le_bytes());
        central.extend_from_slice(&nlen.to_le_bytes());
        central.extend_from_slice(&0u16.to_le_bytes()); // extra
        central.extend_from_slice(&0u16.to_le_bytes()); // comment
        central.extend_from_slice(&0u16.to_le_bytes()); // disk
        central.extend_from_slice(&0u16.to_le_bytes()); // int attr
        central.extend_from_slice(&0u32.to_le_bytes()); // ext attr
        central.extend_from_slice(&offset.to_le_bytes());
        central.extend_from_slice(f.name.as_bytes());
        count += 1;
    }
    let cd_offset = out.len() as u32;
    out.append(&mut central);
    let cd_size = out.len() as u32 - cd_offset;
    out.extend_from_slice(&[0x50, 0x4b, 0x05, 0x06]);
    out.extend_from_slice(&0u16.to_le_bytes());
    out.extend_from_slice(&0u16.to_le_bytes());
    out.extend_from_slice(&count.to_le_bytes());
    out.extend_from_slice(&count.to_le_bytes());
    out.extend_from_slice(&cd_size.to_le_bytes());
    out.extend_from_slice(&cd_offset.to_le_bytes());
    out.extend_from_slice(&0u16.to_le_bytes());
    out
}

// ═══════════════════════ 单元测试 ═══════════════════════

#[cfg(test)]
mod tests {
    use super::*;

    fn sample_html() -> String {
        format!(
            "<html><head><style>x{{y:z}}</style></head><body>\
<div class=WordSection1><!-- t --><h1>标题一二</h1>\
<p>普通段落 <b>粗体</b> 与 <sub>下标</sub> &nbsp;&amp;&nbsp; 符号</p>\
<p class='note center'>居中注记</p>\
<table align=\"center\"><thead><tr><th>参数</th><th>值</th></tr></thead>\
<tr><td>风量</td><td>123 <sup>[<a href=\"#eq7\">7</a>]</sup></td></tr></table>\
<h2>附录</h2><p><a name=\"eq7\">公式七</a></p>\
<p style='font-family:Consolas;text-align:center'>0123456789abcdef</p>\
</div></body></html>"
        )
    }

    #[test]
    fn zip_store_roundtrip_magic() {
        let z = zip_store(&[
            ZipFile {
                name: "a.txt",
                data: b"hello".to_vec(),
            },
            ZipFile {
                name: "b/c.bin",
                data: vec![0, 255, 128, 64],
            },
        ]);
        assert_eq!(&z[0..4], b"PK\x03\x04");
        assert!(z.windows(4).any(|w| w == *b"PK\x05\x06"));
    }

    #[test]
    fn crc32_known() {
        assert_eq!(crc32(b"123456789"), 0xCBF4_3926);
        assert_eq!(crc32(b""), 0);
    }

    #[test]
    fn parse_and_render_subset() {
        let nodes = root_children(&sample_html());
        assert!(!nodes.is_empty());
        let mut cv = Convert::default();
        let body = cv.render_blocks(&nodes);
        assert!(body.contains("<w:tbl>"));
        assert!(body.contains("w:tblHeader")); // thead → 跨页重复表头
        assert!(body.contains("w:shd")); // th 底纹
        assert!(body.contains("<w:vertAlign w:val=\"subscript\"/>"));
        assert!(body.contains("<w:hyperlink w:anchor=\"eq7\">"));
        assert!(body.contains("w:bookmarkStart"));
        assert!(body.contains("Consolas"));
        assert!(body.contains("w:jc w:val=\"center\""));
    }

    #[test]
    fn source_part_roundtrip_and_extract() {
        let html = sample_html();
        let bytes = build_docx(&html, "2026-11-25", &html);
        assert_eq!(&bytes[0..2], b"PK");
        let back = extract_source_docx(&bytes).expect("source.html 应可抽出");
        assert_eq!(back, html);
    }

    #[test]
    fn header_footer_parts_present() {
        let html = sample_html();
        let bytes = build_docx(&html, "2026-11-25", &html);
        let s = String::from_utf8_lossy(&bytes).into_owned();
        assert!(s.contains("养殖用房屋设备自动化计算系统"));
        assert!(s.contains("NUMPAGES"));
        assert!(s.contains("headerReference"));
        // 中文页脚公司名
        assert!(s.contains("青岛鑫光正钢结构股份有限公司"));
    }

    #[test]
    fn base64_decode_known() {
        assert_eq!(b64_decode("aGVsbG8=").unwrap(), b"hello");
        assert_eq!(b64_decode("AAECAwQ=").unwrap(), vec![0, 1, 2, 3, 4]);
    }
}
