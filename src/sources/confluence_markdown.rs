/// Converts Confluence storage format (XHTML) into Markdown.
///
/// This is a pure function with no IO or async. It implements a lightweight
/// tokenizer and recursive walker to handle Confluence-specific tags like
/// `ac:structured-macro`, `ac:parameter`, and CDATA sections.

// ── Tokenizer ───────────────────────────────────────────────────────────────

#[derive(Debug, Clone)]
enum Token {
    OpenTag {
        name: String,
        attrs: Vec<(String, String)>,
    },
    CloseTag {
        name: String,
    },
    SelfClosingTag {
        name: String,
        attrs: Vec<(String, String)>,
    },
    Text(String),
    CData(String),
}

fn tokenize(input: &str) -> Vec<Token> {
    let mut tokens = Vec::new();
    let chars: Vec<char> = input.chars().collect();
    let len = chars.len();
    let mut i = 0;

    while i < len {
        if chars[i] == '<' {
            // Check for CDATA
            if i + 8 < len && &input[i..i + 9] == "<![CDATA[" {
                let start = i + 9;
                if let Some(end) = input[start..].find("]]>") {
                    tokens.push(Token::CData(input[start..start + end].to_string()));
                    i = start + end + 3;
                } else {
                    // Unterminated CDATA — treat rest as text
                    tokens.push(Token::CData(input[start..].to_string()));
                    break;
                }
                continue;
            }

            // Check for comment <!-- ... -->
            if i + 3 < len && &input[i..i + 4] == "<!--" {
                if let Some(end) = input[i + 4..].find("-->") {
                    i = i + 4 + end + 3;
                } else {
                    break;
                }
                continue;
            }

            // Find the end of this tag
            let tag_start = i;
            i += 1;
            // Handle quoted attribute values that may contain '>'
            let mut in_quote: Option<char> = None;
            while i < len {
                if let Some(q) = in_quote {
                    if chars[i] == q {
                        in_quote = None;
                    }
                } else if chars[i] == '"' || chars[i] == '\'' {
                    in_quote = Some(chars[i]);
                } else if chars[i] == '>' {
                    break;
                }
                i += 1;
            }
            if i >= len {
                // Unterminated tag — discard
                break;
            }
            let tag_content = &input[tag_start + 1..i]; // between < and >
            i += 1; // skip '>'

            if let Some(stripped) = tag_content.strip_prefix('/') {
                // Close tag
                let name = stripped.trim().to_lowercase();
                tokens.push(Token::CloseTag { name });
            } else if tag_content.ends_with('/') {
                // Self-closing tag
                let inner = &tag_content[..tag_content.len() - 1];
                let (name, attrs) = parse_tag_inner(inner);
                tokens.push(Token::SelfClosingTag { name, attrs });
            } else {
                let (name, attrs) = parse_tag_inner(tag_content);
                // Some tags are inherently void (no close tag in HTML)
                if is_void_element(&name) {
                    tokens.push(Token::SelfClosingTag { name, attrs });
                } else {
                    tokens.push(Token::OpenTag { name, attrs });
                }
            }
        } else {
            // Text content
            let start = i;
            while i < len && chars[i] != '<' {
                i += 1;
            }
            let text = &input[start..i];
            if !text.is_empty() {
                tokens.push(Token::Text(decode_entities(text)));
            }
        }
    }

    tokens
}

fn is_void_element(name: &str) -> bool {
    matches!(name, "br" | "hr" | "img" | "input" | "meta" | "link")
}

fn parse_tag_inner(s: &str) -> (String, Vec<(String, String)>) {
    let s = s.trim();
    let mut attrs = Vec::new();

    // Extract tag name (first whitespace-delimited token)
    let name_end = s.find(|c: char| c.is_whitespace()).unwrap_or(s.len());
    let name = s[..name_end].to_lowercase();
    let rest = s[name_end..].trim();

    if !rest.is_empty() {
        parse_attributes(rest, &mut attrs);
    }

    (name, attrs)
}

fn parse_attributes(s: &str, attrs: &mut Vec<(String, String)>) {
    let chars: Vec<char> = s.chars().collect();
    let len = chars.len();
    let mut i = 0;

    while i < len {
        // Skip whitespace
        while i < len && chars[i].is_whitespace() {
            i += 1;
        }
        if i >= len {
            break;
        }

        // Read attribute name
        let name_start = i;
        while i < len && chars[i] != '=' && !chars[i].is_whitespace() {
            i += 1;
        }
        let attr_name = s[name_start..i].to_string();
        if attr_name.is_empty() {
            break;
        }

        // Skip whitespace around '='
        while i < len && chars[i].is_whitespace() {
            i += 1;
        }
        if i >= len || chars[i] != '=' {
            // Boolean attribute with no value
            attrs.push((attr_name, String::new()));
            continue;
        }
        i += 1; // skip '='
        while i < len && chars[i].is_whitespace() {
            i += 1;
        }

        // Read attribute value
        if i < len && (chars[i] == '"' || chars[i] == '\'') {
            let quote = chars[i];
            i += 1;
            let val_start = i;
            while i < len && chars[i] != quote {
                i += 1;
            }
            let val = &s[val_start..i];
            attrs.push((attr_name, decode_entities(val)));
            if i < len {
                i += 1; // skip closing quote
            }
        } else {
            // Unquoted value
            let val_start = i;
            while i < len && !chars[i].is_whitespace() {
                i += 1;
            }
            attrs.push((attr_name, s[val_start..i].to_string()));
        }
    }
}

fn decode_entities(s: &str) -> String {
    // Fast path: skip 7 String allocations when no entities present
    if !s.contains('&') {
        return s.to_string();
    }
    s.replace("&amp;", "&")
        .replace("&lt;", "<")
        .replace("&gt;", ">")
        .replace("&quot;", "\"")
        .replace("&#39;", "'")
        .replace("&apos;", "'")
        .replace("&nbsp;", " ")
}

fn get_attr<'a>(attrs: &'a [(String, String)], key: &str) -> Option<&'a str> {
    attrs
        .iter()
        .find(|(k, _)| k == key)
        .map(|(_, v)| v.as_str())
}

// ── Walker / Converter ──────────────────────────────────────────────────────

#[derive(Clone, Copy, PartialEq)]
enum ListType {
    Ul,
    Ol,
}

struct Walker {
    tokens: Vec<Token>,
    pos: usize,
    output: String,
    list_stack: Vec<ListType>,
    ol_counters: Vec<usize>,
    in_pre: bool,
    in_code: bool,
    // Table state
    in_table: bool,
    table_rows: Vec<Vec<String>>,
    current_row: Vec<String>,
    current_cell: String,
    in_cell: bool,
    has_header: bool,
    // Structured macro state
    macro_name: String,
    macro_depth: usize,
    collecting_macro: bool,
    macro_tokens: Vec<Token>,
}

impl Walker {
    fn new(tokens: Vec<Token>) -> Self {
        Walker {
            tokens,
            pos: 0,
            output: String::new(),
            list_stack: Vec::new(),
            ol_counters: Vec::new(),
            in_pre: false,
            in_code: false,
            in_table: false,
            table_rows: Vec::new(),
            current_row: Vec::new(),
            current_cell: String::new(),
            in_cell: false,
            has_header: false,
            macro_name: String::new(),
            macro_depth: 0,
            collecting_macro: false,
            macro_tokens: Vec::new(),
        }
    }

    fn walk(&mut self) {
        while self.pos < self.tokens.len() {
            let token = self.tokens[self.pos].clone();
            self.pos += 1;
            self.process_token(token);
        }
    }

    fn process_token(&mut self, token: Token) {
        // If we're collecting tokens for a structured macro, accumulate them
        if self.collecting_macro {
            match &token {
                Token::OpenTag { name, .. } if name == "ac:structured-macro" => {
                    self.macro_depth += 1;
                    self.macro_tokens.push(token);
                    return;
                }
                Token::CloseTag { name } if name == "ac:structured-macro" => {
                    if self.macro_depth > 0 {
                        self.macro_depth -= 1;
                        self.macro_tokens.push(token);
                        return;
                    }
                    // End of our macro — process it
                    self.collecting_macro = false;
                    let macro_name = self.macro_name.clone();
                    let collected = std::mem::take(&mut self.macro_tokens);
                    self.process_structured_macro(&macro_name, collected);
                    return;
                }
                _ => {
                    self.macro_tokens.push(token);
                    return;
                }
            }
        }

        match token {
            Token::Text(ref text) => {
                if self.in_cell {
                    self.current_cell.push_str(text);
                } else if self.in_pre || self.in_code {
                    self.output.push_str(text);
                } else {
                    self.output.push_str(text);
                }
            }
            Token::CData(ref content) => {
                if self.in_cell {
                    self.current_cell.push_str(content);
                } else {
                    self.output.push_str(content);
                }
            }
            Token::SelfClosingTag { ref name, ref attrs } => {
                self.handle_self_closing(name, attrs);
            }
            Token::OpenTag { ref name, ref attrs } => {
                self.handle_open_tag(name, attrs);
            }
            Token::CloseTag { ref name } => {
                self.handle_close_tag(name);
            }
        }
    }

    fn handle_self_closing(&mut self, name: &str, attrs: &[(String, String)]) {
        match name {
            "br" => {
                if self.in_cell {
                    self.current_cell.push('\n');
                } else {
                    self.output.push('\n');
                }
            }
            "hr" => {
                self.output.push_str("---\n\n");
            }
            "img" => {
                let alt = get_attr(attrs, "alt").unwrap_or("");
                let src = get_attr(attrs, "src").unwrap_or("");
                let md = format!("![{}]({})", alt, src);
                if self.in_cell {
                    self.current_cell.push_str(&md);
                } else {
                    self.output.push_str(&md);
                }
            }
            _ => {}
        }
    }

    fn handle_open_tag(&mut self, name: &str, attrs: &[(String, String)]) {
        match name {
            "h1" | "h2" | "h3" | "h4" | "h5" | "h6" => {
                let level = name[1..].parse::<usize>().unwrap_or(1);
                for _ in 0..level {
                    self.output.push('#');
                }
                self.output.push(' ');
            }
            "p" => {
                // Just mark start; we add \n\n on close
            }
            "strong" | "b" => {
                if self.in_cell {
                    self.current_cell.push_str("**");
                } else {
                    self.output.push_str("**");
                }
            }
            "em" | "i" => {
                if self.in_cell {
                    self.current_cell.push('*');
                } else {
                    self.output.push('*');
                }
            }
            "del" | "s" => {
                if self.in_cell {
                    self.current_cell.push_str("~~");
                } else {
                    self.output.push_str("~~");
                }
            }
            "a" => {
                // We need to collect inner text until </a>
                // For simplicity, we push a marker and handle on close
                let href = get_attr(attrs, "href").unwrap_or("").to_string();
                // Push a marker into output that we'll replace on close
                self.output.push_str(&format!("\x01LINK:{}\x02", href));
            }
            "ul" => {
                // If nested inside a <li>, ensure we start on a new line
                if !self.list_stack.is_empty() && !self.output.ends_with('\n') {
                    self.output.push('\n');
                }
                self.list_stack.push(ListType::Ul);
            }
            "ol" => {
                if !self.list_stack.is_empty() && !self.output.ends_with('\n') {
                    self.output.push('\n');
                }
                self.list_stack.push(ListType::Ol);
                self.ol_counters.push(0);
            }
            "li" => {
                let depth = self.list_stack.len().saturating_sub(1);
                let list_type = self.list_stack.last().copied().unwrap_or(ListType::Ul);

                match list_type {
                    ListType::Ul => {
                        let indent = "  ".repeat(depth);
                        self.output.push_str(&indent);
                        self.output.push_str("- ");
                    }
                    ListType::Ol => {
                        if let Some(counter) = self.ol_counters.last_mut() {
                            *counter += 1;
                            let n = *counter;
                            let indent = "   ".repeat(depth);
                            self.output.push_str(&indent);
                            self.output.push_str(&format!("{}. ", n));
                        }
                    }
                }
            }
            "code" => {
                if !self.in_pre {
                    self.in_code = true;
                    if self.in_cell {
                        self.current_cell.push('`');
                    } else {
                        self.output.push('`');
                    }
                }
            }
            "pre" => {
                self.in_pre = true;
                self.output.push_str("```\n");
            }
            "table" => {
                self.in_table = true;
                self.table_rows.clear();
                self.has_header = false;
            }
            "tr" => {
                self.current_row.clear();
            }
            "th" => {
                self.in_cell = true;
                self.current_cell.clear();
                self.has_header = true;
            }
            "td" => {
                self.in_cell = true;
                self.current_cell.clear();
            }
            "ac:structured-macro" => {
                let macro_name = get_attr(attrs, "ac:name").unwrap_or("").to_string();
                self.macro_name = macro_name;
                self.collecting_macro = true;
                self.macro_depth = 0;
                self.macro_tokens.clear();
            }
            // Tags we just ignore (strip tag, keep content)
            _ => {}
        }
    }

    fn handle_close_tag(&mut self, name: &str) {
        match name {
            "h1" | "h2" | "h3" | "h4" | "h5" | "h6" => {
                self.output.push('\n');
            }
            "p" => {
                if self.in_cell {
                    // Inside a table cell, paragraphs just add a space
                    self.current_cell.push(' ');
                } else {
                    self.output.push_str("\n\n");
                }
            }
            "strong" | "b" => {
                if self.in_cell {
                    self.current_cell.push_str("**");
                } else {
                    self.output.push_str("**");
                }
            }
            "em" | "i" => {
                if self.in_cell {
                    self.current_cell.push('*');
                } else {
                    self.output.push('*');
                }
            }
            "del" | "s" => {
                if self.in_cell {
                    self.current_cell.push_str("~~");
                } else {
                    self.output.push_str("~~");
                }
            }
            "a" => {
                // Find the marker and replace
                if let Some(marker_start) = self.output.rfind('\x01') {
                    let marker_and_rest = self.output[marker_start..].to_string();
                    if let Some(colon_pos) = marker_and_rest.find(':') {
                        if let Some(end_pos) = marker_and_rest.find('\x02') {
                            let href = &marker_and_rest[colon_pos + 1..end_pos];
                            let link_text = &marker_and_rest[end_pos + 1..];
                            let md = format!("[{}]({})", link_text, href);
                            self.output.truncate(marker_start);
                            self.output.push_str(&md);
                        }
                    }
                }
            }
            "ul" => {
                self.list_stack.pop();
                if self.list_stack.is_empty() {
                    self.output.push('\n');
                }
            }
            "ol" => {
                self.list_stack.pop();
                self.ol_counters.pop();
                if self.list_stack.is_empty() {
                    self.output.push('\n');
                }
            }
            "li" => {
                // Ensure line ends with newline
                if !self.output.ends_with('\n') {
                    self.output.push('\n');
                }
            }
            "code" => {
                if !self.in_pre {
                    self.in_code = false;
                    if self.in_cell {
                        self.current_cell.push('`');
                    } else {
                        self.output.push('`');
                    }
                }
            }
            "pre" => {
                self.in_pre = false;
                self.output.push_str("\n```\n\n");
            }
            "th" | "td" => {
                self.in_cell = false;
                self.current_row
                    .push(self.current_cell.trim().to_string());
                self.current_cell.clear();
            }
            "tr" => {
                let row = std::mem::take(&mut self.current_row);
                self.table_rows.push(row);
            }
            "table" => {
                self.in_table = false;
                self.flush_table();
            }
            // Ignore close tags for unknown elements and Confluence internal tags
            _ => {}
        }
    }

    fn flush_table(&mut self) {
        let rows = std::mem::take(&mut self.table_rows);
        if rows.is_empty() {
            return;
        }

        let mut iter = rows.into_iter();

        // First row is header if has_header, otherwise just a regular row
        if let Some(first_row) = iter.next() {
            // Emit header
            self.output.push_str("| ");
            self.output.push_str(&first_row.join(" | "));
            self.output.push_str(" |\n");

            if self.has_header {
                // Separator
                self.output.push('|');
                for _ in &first_row {
                    self.output.push_str("---|");
                }
                self.output.push('\n');
            }

            // Remaining rows
            for row in iter {
                self.output.push_str("| ");
                self.output.push_str(&row.join(" | "));
                self.output.push_str(" |\n");
            }
        }

        self.output.push('\n');
    }

    fn process_structured_macro(&mut self, macro_name: &str, tokens: Vec<Token>) {
        // Extract parameters and body from the collected tokens
        let mut params: Vec<(String, String)> = Vec::new();
        let mut body_tokens: Vec<Token> = Vec::new();
        let mut in_param = false;
        let mut param_name = String::new();
        let mut param_value = String::new();
        let mut in_body = false;
        let mut body_depth = 0;

        for tok in &tokens {
            match tok {
                Token::OpenTag { name, attrs } if name == "ac:parameter" => {
                    in_param = true;
                    param_name = get_attr(attrs, "ac:name")
                        .unwrap_or("")
                        .to_string();
                    param_value.clear();
                }
                Token::CloseTag { name } if name == "ac:parameter" && in_param => {
                    in_param = false;
                    params.push((param_name.clone(), param_value.clone()));
                }
                Token::OpenTag { name, .. }
                    if name == "ac:rich-text-body" || name == "ac:plain-text-body" =>
                {
                    in_body = true;
                    body_depth = 0usize;
                }
                Token::CloseTag { name }
                    if (name == "ac:rich-text-body" || name == "ac:plain-text-body")
                        && in_body
                        && body_depth == 0 =>
                {
                    in_body = false;
                }
                _ if in_param => {
                    if let Token::Text(t) = tok {
                        param_value.push_str(t);
                    } else if let Token::CData(c) = tok {
                        param_value.push_str(c);
                    }
                }
                _ if in_body => {
                    // Track depth for nested tags
                    match tok {
                        Token::OpenTag { .. } => body_depth += 1,
                        Token::CloseTag { .. } => {
                            body_depth = body_depth.saturating_sub(1);
                        }
                        _ => {}
                    }
                    body_tokens.push(tok.clone());
                }
                _ => {}
            }
        }

        // Convert body tokens to markdown using a sub-walker
        let body_md = if !body_tokens.is_empty() {
            let mut sub = Walker::new(body_tokens);
            sub.list_stack = Vec::new();
            sub.walk();
            sub.output.trim().to_string()
        } else {
            String::new()
        };

        match macro_name {
            "code" => {
                let lang = params
                    .iter()
                    .find(|(k, _)| k == "language")
                    .map(|(_, v)| v.as_str())
                    .unwrap_or("");

                // For code macros, extract CDATA content directly
                let code_content = self.extract_cdata_from_tokens(&tokens);
                let content = if !code_content.is_empty() {
                    code_content
                } else {
                    body_md
                };

                self.output.push_str(&format!("```{}\n", lang));
                self.output.push_str(&content);
                self.output.push_str("\n```\n\n");
            }
            "info" | "warning" | "note" | "tip" => {
                let label = match macro_name {
                    "info" => "Info",
                    "warning" => "Warning",
                    "note" => "Note",
                    "tip" => "Tip",
                    _ => unreachable!(),
                };
                self.output
                    .push_str(&format!("> **{}:** {}\n\n", label, body_md));
            }
            "expand" => {
                let title = params
                    .iter()
                    .find(|(k, _)| k == "title")
                    .map(|(_, v)| v.as_str())
                    .unwrap_or("Details");

                self.output
                    .push_str(&format!("<details><summary>{}</summary>\n\n", title));
                self.output.push_str(&body_md);
                self.output.push_str("\n\n</details>\n\n");
            }
            _ => {
                // Unknown macro — just emit the body
                if !body_md.is_empty() {
                    self.output.push_str(&body_md);
                }
            }
        }
    }

    fn extract_cdata_from_tokens(&self, tokens: &[Token]) -> String {
        for tok in tokens {
            if let Token::CData(content) = tok {
                return content.clone();
            }
        }
        String::new()
    }
}

// ── Public API ──────────────────────────────────────────────────────────────

/// Convert Confluence storage format HTML to Markdown.
pub fn to_markdown(html: &str) -> String {
    if html.is_empty() {
        return String::new();
    }

    let tokens = tokenize(html);
    let mut walker = Walker::new(tokens);
    walker.walk();

    walker.output
}

#[cfg(test)]
mod unit_tests {
    use super::*;

    #[test]
    fn tokenizer_basic() {
        let tokens = tokenize("<p>hello</p>");
        assert_eq!(tokens.len(), 3);
    }

    #[test]
    fn tokenizer_self_closing() {
        let tokens = tokenize("<br/>");
        assert_eq!(tokens.len(), 1);
        assert!(matches!(&tokens[0], Token::SelfClosingTag { name, .. } if name == "br"));
    }

    #[test]
    fn tokenizer_cdata() {
        let tokens = tokenize("<![CDATA[hello world]]>");
        assert_eq!(tokens.len(), 1);
        assert!(matches!(&tokens[0], Token::CData(s) if s == "hello world"));
    }

    #[test]
    fn tokenizer_attributes() {
        let tokens = tokenize(r#"<a href="https://example.com" class="link">text</a>"#);
        assert_eq!(tokens.len(), 3);
        if let Token::OpenTag { attrs, .. } = &tokens[0] {
            assert_eq!(get_attr(attrs, "href"), Some("https://example.com"));
            assert_eq!(get_attr(attrs, "class"), Some("link"));
        } else {
            panic!("Expected OpenTag");
        }
    }
}
