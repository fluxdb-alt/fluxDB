// 内置产品文档索引：编译期打包操作手册，供 AI 的 docs.search / docs.read 使用。
//
// 只读本地静态内容：不扫描用户目录、不联网、不读取连接或凭据。分节按 Markdown
// 标题层级切分；父节正文包含子节，模型一次读取即可看到完整操作步骤，子节仍可单独检索。

/// 操作手册随二进制发布，不依赖安装目录或用户附件授权。
const USER_GUIDE_MARKDOWN: &str =
    include_str!(concat!(env!("CARGO_MANIFEST_DIR"), "/../../docs/user-guide.md"));

pub const USER_GUIDE_SOURCE: &str = "user-guide";

struct DocSourceDef {
    id: &'static str,
    title: &'static str,
    markdown: &'static str,
}

const DOC_SOURCES: &[DocSourceDef] = &[DocSourceDef {
    id: USER_GUIDE_SOURCE,
    title: "FluxDB 操作手册",
    markdown: USER_GUIDE_MARKDOWN,
}];

/// 一个可检索的 Markdown 小节。
struct DocSection {
    id: String,
    title: String,
    /// 最近的二级标题（章）；二级标题自身也填自己。
    chapter: String,
    /// 小节正文（含子节），不含标题行。
    content: String,
    /// 标题与正文的小写副本，避免每次检索重复转换。
    search_text: String,
    /// 文档中的原始顺序，用于同分排序。
    order: usize,
}

struct DocDocument {
    id: &'static str,
    title: &'static str,
    sections: Vec<DocSection>,
}

static DOC_INDEX: std::sync::OnceLock<Vec<DocDocument>> = std::sync::OnceLock::new();

fn docs_index() -> &'static [DocDocument] {
    DOC_INDEX.get_or_init(|| {
        DOC_SOURCES
            .iter()
            .map(|source| DocDocument {
                id: source.id,
                title: source.title,
                sections: parse_doc_sections(source.markdown),
            })
            .collect()
    })
}

struct Heading {
    line_index: usize,
    level: u8,
    title: String,
}

/// 解析 Markdown 标题；跳过目录、代码块内的伪标题与一级标题。
fn parse_doc_sections(markdown: &str) -> Vec<DocSection> {
    let lines: Vec<&str> = markdown.lines().collect();
    let headings = collect_headings(&lines);
    let mut sections = Vec::with_capacity(headings.len());
    let mut current_chapter = String::new();
    let mut seen_ids = std::collections::HashMap::<String, usize>::new();

    for (index, heading) in headings.iter().enumerate() {
        if heading.level == 2 {
            current_chapter = heading.title.clone();
        }
        let chapter = if heading.level == 2 {
            heading.title.clone()
        } else {
            current_chapter.clone()
        };
        let end_line = headings
            .iter()
            .skip(index + 1)
            .find(|next| next.level <= heading.level)
            .map(|next| next.line_index)
            .unwrap_or(lines.len());
        let content = lines[heading.line_index + 1..end_line]
            .join("\n")
            .trim()
            .to_string();
        let base_id = heading_id(&heading.title, index);
        let count = seen_ids.entry(base_id.clone()).or_insert(0);
        *count += 1;
        let id = if *count == 1 {
            base_id
        } else {
            format!("{base_id}-{}", *count)
        };
        let search_text = format!("{} {} {}", heading.title, chapter, content).to_lowercase();
        sections.push(DocSection {
            id,
            title: heading.title.clone(),
            chapter,
            content,
            search_text,
            order: index,
        });
    }
    sections
}

fn collect_headings(lines: &[&str]) -> Vec<Heading> {
    let mut headings = Vec::new();
    let mut in_fence = false;
    for (line_index, line) in lines.iter().enumerate() {
        let trimmed = line.trim_start();
        if trimmed.starts_with("```") || trimmed.starts_with("~~~") {
            in_fence = !in_fence;
            continue;
        }
        if in_fence {
            continue;
        }
        let level = trimmed.chars().take_while(|ch| *ch == '#').count();
        if !(2..=4).contains(&level) {
            continue;
        }
        let title = trimmed[level..].trim();
        // 目录只含链接，不提供可执行步骤，避免污染检索结果。
        if title.is_empty() || title == "目录" {
            continue;
        }
        headings.push(Heading {
            line_index,
            level: level as u8,
            title: title.to_string(),
        });
    }
    headings
}

/// 优先使用标题前的章节号（`8.1`、`12.4`）；没有编号时退化为序号 ID。
fn heading_id(title: &str, index: usize) -> String {
    let token = title.split_whitespace().next().unwrap_or("");
    let candidate = token.trim_end_matches('.');
    let numbered = !candidate.is_empty()
        && candidate.chars().all(|ch| ch.is_ascii_digit() || ch == '.')
        && !candidate.starts_with('.');
    if numbered {
        candidate.to_string()
    } else {
        format!("section-{}", index + 1)
    }
}

/// 搜索结果；只返回定位信息与短摘要，正文由 docs.read 按需读取。
pub struct DocSearchHit {
    pub source: &'static str,
    pub source_title: &'static str,
    pub section_id: String,
    pub title: String,
    pub chapter: String,
    pub snippet: String,
}

/// 检索内置文档。`source` 为空时使用第一个文档源。
pub fn docs_search(
    source: Option<&str>,
    query: &str,
    limit: usize,
) -> Result<Vec<DocSearchHit>, String> {
    let document = resolve_document(source)?;
    let query = query.trim().to_lowercase();
    if query.is_empty() {
        return Ok(Vec::new());
    }
    let terms: Vec<&str> = query.split_whitespace().filter(|term| !term.is_empty()).collect();
    let mut hits: Vec<(i32, &DocSection)> = document
        .sections
        .iter()
        .filter_map(|section| {
            let score = section_score(section, &query, &terms);
            (score > 0).then_some((score, section))
        })
        .collect();
    hits.sort_by(|left, right| {
        right
            .0
            .cmp(&left.0)
            .then_with(|| left.1.order.cmp(&right.1.order))
    });
    let limit = limit.clamp(1, 10);
    Ok(hits
        .into_iter()
        .take(limit)
        .map(|(_, section)| DocSearchHit {
            source: document.id,
            source_title: document.title,
            section_id: section.id.clone(),
            title: section.title.clone(),
            chapter: section.chapter.clone(),
            snippet: section_snippet(&section.content, &query, &terms),
        })
        .collect())
}

fn section_score(section: &DocSection, query: &str, terms: &[&str]) -> i32 {
    let title = section.title.to_lowercase();
    let mut score = 0;
    if title == query {
        score += 1000;
    } else if title.contains(query) {
        score += 300;
    }
    let mut matched_terms = 0;
    for term in terms {
        if title.contains(term) {
            score += 60;
            matched_terms += 1;
        }
    }
    if section.search_text.contains(query) {
        score += 40;
    }
    for term in terms {
        if section.search_text.contains(term) {
            score += 5;
            matched_terms += 1;
        }
    }
    // 多词查询尽量要求全部命中；只命中一个词的章节仍保留，但排在后面。
    if matched_terms >= terms.len() && terms.len() > 1 {
        score += 100;
    }
    score
}

/// 命中词前后各取一段，压成单行，避免把整节内容塞进搜索结果。
fn section_snippet(content: &str, query: &str, terms: &[&str]) -> String {
    let lower = content.to_lowercase();
    let needle = std::iter::once(query)
        .chain(terms.iter().copied())
        .find(|needle| !needle.is_empty() && lower.contains(*needle))
        .unwrap_or("");
    let hit_byte = if needle.is_empty() {
        0
    } else {
        lower.find(needle).unwrap_or(0)
    };
    // lower 与原文的字符边界一致（手册以中文/ASCII 为主），按字符数定位可避免切坏 UTF-8。
    let hit_char = lower[..hit_byte].chars().count();
    let chars: Vec<char> = content.chars().collect();
    let start = hit_char.saturating_sub(60);
    let end = (hit_char + needle.chars().count() + 100).min(chars.len());
    let mut snippet: String = chars[start..end].iter().collect();
    snippet = snippet.split_whitespace().collect::<Vec<_>>().join(" ");
    if start > 0 {
        snippet.insert(0, '…');
    }
    if end < chars.len() {
        snippet.push('…');
    }
    snippet
}

/// 按字节分页读取一个小节；返回的 offset/next_offset 均为 UTF-8 字符边界。
pub struct DocRead {
    pub source: &'static str,
    pub source_title: &'static str,
    pub section_id: String,
    pub title: String,
    pub chapter: String,
    pub offset: usize,
    pub next_offset: Option<usize>,
    pub truncated: bool,
    pub text: String,
}

pub fn docs_read(
    source: Option<&str>,
    section_id: &str,
    offset: usize,
    max_bytes: usize,
) -> Result<DocRead, String> {
    let document = resolve_document(source)?;
    let section = document
        .sections
        .iter()
        .find(|section| section.id == section_id)
        .ok_or_else(|| format!("未找到文档小节: {section_id}"))?;
    let max_bytes = max_bytes.clamp(1, 16 * 1024);
    let offset = floor_char_boundary(&section.content, offset.min(section.content.len()));
    let end = ceil_char_boundary(
        &section.content,
        (offset + max_bytes).min(section.content.len()),
    );
    Ok(DocRead {
        source: document.id,
        source_title: document.title,
        section_id: section.id.clone(),
        title: section.title.clone(),
        chapter: section.chapter.clone(),
        offset,
        next_offset: (end < section.content.len()).then_some(end),
        truncated: end < section.content.len(),
        text: section.content[offset..end].to_string(),
    })
}

fn resolve_document(source: Option<&str>) -> Result<&'static DocDocument, String> {
    let documents = docs_index();
    match source {
        Some(id) => documents
            .iter()
            .find(|document| document.id == id)
            .ok_or_else(|| format!("未找到文档源: {id}")),
        None => documents.first().ok_or_else(|| "没有可用的内置文档".to_string()),
    }
}

fn floor_char_boundary(text: &str, mut offset: usize) -> usize {
    offset = offset.min(text.len());
    while offset > 0 && !text.is_char_boundary(offset) {
        offset -= 1;
    }
    offset
}

fn ceil_char_boundary(text: &str, mut offset: usize) -> usize {
    offset = offset.min(text.len());
    while offset < text.len() && !text.is_char_boundary(offset) {
        offset += 1;
    }
    offset
}

#[cfg(test)]
mod manual_docs_tests {
    use super::*;

    const SAMPLE: &str = "# 标题\n\n## 目录\n- [1. 开始](#1)\n\n## 1. 开始\n说明\n### 1.1 子节\n细节\n```\n## 代码块里的伪标题\n```\n#### 1.1.1 更深\n更多细节\n## 2. 结束\n尾\n";

    #[test]
    fn parses_sections_and_skips_toc_and_code_fences() {
        let sections = parse_doc_sections(SAMPLE);
        let ids: Vec<&str> = sections.iter().map(|section| section.id.as_str()).collect();
        assert_eq!(ids, vec!["1", "1.1", "1.1.1", "2"]);
        let first = &sections[0];
        assert_eq!(first.chapter, "1. 开始");
        assert!(first.content.contains("1.1 子节"));
        assert!(first.content.contains("更多细节"));
        assert!(sections.iter().all(|section| !section.title.contains("伪标题")));
        assert_eq!(sections[1].chapter, "1. 开始");
        assert_eq!(sections[1].content, "细节\n```\n## 代码块里的伪标题\n```\n#### 1.1.1 更深\n更多细节");
    }

    #[test]
    fn bundled_user_guide_is_searchable_and_readable() {
        let hits = docs_search(None, "MariaDB", 5).expect("search");
        assert!(!hits.is_empty());
        assert!(hits.iter().any(|hit| hit.section_id.starts_with("2.")));
        let read = docs_read(None, "8.1", 0, 8192).expect("read");
        assert!(read.text.contains("备份目录"));
        assert_eq!(read.source, USER_GUIDE_SOURCE);
    }

    #[test]
    fn read_pages_do_not_split_utf8() {
        let mut offset = 0;
        let mut text = String::new();
        loop {
            let page = docs_read(None, "15.1", offset, 17).expect("read");
            assert!(!page.text.contains('\u{fffd}'));
            text.push_str(&page.text);
            match page.next_offset {
                Some(next) => offset = next,
                None => break,
            }
        }
        assert!(text.contains("连接密码"));
    }

    #[test]
    fn unknown_source_and_section_are_rejected() {
        assert!(docs_search(Some("missing"), "备份", 3).is_err());
        assert!(docs_read(None, "999.999", 0, 1024).is_err());
    }
}
