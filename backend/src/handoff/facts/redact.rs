/* ------------------------------ 敏感信息遮蔽 ------------------------------ */
//
// 结构：几遍互相独立的线性扫描各自收集「要遮的区间」，最后排序合并、一次输出。
// 每遍扫描里，任何一次向前查看都有长度上限，或者扫描后整段跳过，保证整体线性（病态输入也不会退化成 O(n²)）。

pub(crate) const MASK: &str = "***";
pub(crate) use crate::infra::redact::{PREFIX_KEYS, PREFIX_MIN};
/// 键名最长这么多字，更长的标识符不当作键名（同时保证扫描线性）
pub(crate) const IDENT_MAX: usize = 128;
/// 单个值最多向前取这么多字
pub(crate) const VALUE_MAX: usize = 4_096;
/// 键值形式的最短值：一般凭证 8 位，密码类 6 位
pub(crate) const SECRET_MIN: usize = 8;
pub(crate) const PASSWORD_MIN: usize = 6;
/// 长串兜底：总长 ≥ 32，且最长的连续字母数字段 ≥ 20
pub(crate) const LONG_MIN: usize = 32;
pub(crate) const LONG_SEGMENT_MIN: usize = 20;
/// 值是这些类型名时不遮（`password: string`）
pub(crate) const TYPE_WORDS: [&str; 20] = [
    "string", "number", "boolean", "bool", "str", "null", "none", "undefined", "true", "false", "optional", "option", "object", "any", "bytes", "text", "required", "varchar", "secretstring", "securestring",
];
/// `…_key` 前面是这些词时不是凭证（`primary_key`、`sort_key`）
pub(crate) const KEY_DENY_BEFORE: [&str; 18] = ["primary", "foreign", "sort", "partition", "cache", "unique", "composite", "lookup", "hash", "map", "dict", "index", "row", "sequence", "hot", "shortcut", "group", "route"];
/// 不带分隔符的复合键名（小写后以它们结尾即算）
pub(crate) const COMPOUND_KEYS: [&str; 15] = ["apikey", "secretkey", "privatekey", "accesskey", "authkey", "signingkey", "masterkey", "licensekey", "clientsecret", "accesstoken", "refreshtoken", "idtoken", "authtoken", "sessiontoken", "apitoken"];
pub(crate) const SCHEMES: [&str; 3] = ["bearer ", "basic ", "token "];

#[derive(Debug, Clone, Copy, PartialEq)]
pub(crate) enum KeyKind {
    /// password / passwd / pass / pwd：值 ≥ 6 位即遮，纯数字也遮
    Password,
    /// api key、token、secret、credential、auth / authoriz*、`…_key`
    Secret,
    /// Cookie / Set-Cookie：取到行尾
    Cookie,
}

pub(crate) fn ident_char(c: char) -> bool {
    c.is_ascii_alphanumeric() || matches!(c, '_' | '-' | '.')
}

pub(crate) fn value_char(c: char) -> bool {
    !c.is_whitespace() && !matches!(c, '"' | '\'' | '`' | ',' | ';' | '(' | ')' | '[' | ']' | '{' | '}' | '<' | '>')
}

/// 长串兜底的字符集（`=` 只作为结尾的 base64 填充，避免把 `KEY=值` 连成一段）
pub(crate) fn long_char(c: char) -> bool {
    c.is_ascii_alphanumeric() || matches!(c, '+' | '/' | '_' | '-')
}

pub(crate) fn jwt_char(c: char) -> bool {
    c.is_ascii_alphanumeric() || matches!(c, '_' | '-')
}

pub(crate) fn is_quote(c: char) -> bool {
    matches!(c, '"' | '\'')
}

/// 键名切成小写的段：按 `_ - .` 与驼峰边界（`XApiKey` → x / api / key）
pub(crate) fn key_segments(ident: &str) -> Vec<String> {
    let chars: Vec<char> = ident.chars().collect();
    let mut out: Vec<String> = Vec::new();
    let mut cur = String::new();
    for (k, &c) in chars.iter().enumerate() {
        if matches!(c, '_' | '-' | '.') {
            if !cur.is_empty() {
                out.push(std::mem::take(&mut cur));
            }
            continue;
        }
        let prev = k.checked_sub(1).map(|p| chars[p]);
        let next = chars.get(k + 1).copied();
        let lower_to_upper = c.is_ascii_uppercase() && prev.is_some_and(|p| p.is_ascii_lowercase() || p.is_ascii_digit());
        let upper_run_end = c.is_ascii_uppercase() && prev.is_some_and(|p| p.is_ascii_uppercase()) && next.is_some_and(|n| n.is_ascii_lowercase());
        if (lower_to_upper || upper_run_end) && !cur.is_empty() {
            out.push(std::mem::take(&mut cur));
        }
        cur.push(c.to_ascii_lowercase());
    }
    if !cur.is_empty() {
        out.push(cur);
    }
    out
}

/// 键名是不是凭证键：`auth` 只认独立成段的 `auth` 与 `authoriz*`（`author`、`oauth_callback`、`authority` 不算）；
/// `token` 只认独立的 token 段（`tokenizer`、`max_tokens` 不算）
pub(crate) fn key_kind(ident: &str) -> Option<KeyKind> {
    let segs = key_segments(ident);
    let last = segs.last()?;
    let joined: String = segs.concat();
    // 不带分隔符的全大写键名不会被切开（`PGPASSWORD`、`REDISPASSWORD`），所以段内包含 password / passwd 即算
    if segs.iter().any(|s| matches!(s.as_str(), "pass" | "pwd" | "passphrase") || s.contains("password") || s.contains("passwd")) {
        return Some(KeyKind::Password);
    }
    if segs.iter().any(|s| s == "cookie") {
        return Some(KeyKind::Cookie);
    }
    // 同理 `GITHUBTOKEN`、`CLIENTSECRET`：段比关键词长、并以它结尾也算（`tokenizer`、`max_tokens` 不以 token 结尾）
    let glued = |s: &str, word: &str| s.len() > word.len() && s.ends_with(word);
    if segs.iter().any(|s| {
        matches!(s.as_str(), "secret" | "secrets" | "token" | "credential" | "credentials" | "auth" | "apikey" | "bearer")
            || s.starts_with("authoriz")
            || glued(s, "token")
            || glued(s, "secret")
    }) {
        return Some(KeyKind::Secret);
    }
    let deny = segs.len() >= 2 && KEY_DENY_BEFORE.contains(&segs[segs.len() - 2].as_str());
    if (last == "key" && !deny) || COMPOUND_KEYS.iter().any(|k| joined.ends_with(k)) {
        return Some(KeyKind::Secret);
    }
    None
}

/// 值要不要遮：长度够；一般凭证另外放过纯数字（`"input_tokens": 12345678`）与纯字母的驼峰标识符
/// （`CancellationToken`、`HeaderValue`、`process.env.OPENAI_API_KEY`）。带 `Bearer` / `Basic` 前缀的值只看长度
pub(crate) fn should_mask(kind: KeyKind, value: &[char], schemed: bool) -> bool {
    let text: String = value.iter().collect();
    let trimmed = text.trim();
    if trimmed.contains(MASK) {
        return false;
    }
    let len = trimmed.chars().count();
    if schemed {
        return len >= PASSWORD_MIN;
    }
    if TYPE_WORDS.contains(&trimmed.to_ascii_lowercase().as_str()) {
        return false;
    }
    match kind {
        KeyKind::Password => len >= PASSWORD_MIN,
        KeyKind::Secret | KeyKind::Cookie => {
            let numeric = trimmed.chars().all(|c| c.is_ascii_digit());
            let identifier = trimmed.chars().all(|c| c.is_ascii_alphabetic() || matches!(c, '.' | '_'))
                && trimmed.chars().any(|c| c.is_ascii_uppercase())
                && trimmed.chars().any(|c| c.is_ascii_lowercase());
            len >= SECRET_MIN && !numeric && !identifier
        }
    }
}

pub(crate) struct Scan<'a> {
    pub(crate) chars: &'a [char],
    pub(crate) lower: Vec<char>,
}

impl Scan<'_> {
    pub(crate) fn len(&self) -> usize {
        self.chars.len()
    }
    pub(crate) fn at(&self, i: usize) -> Option<char> {
        self.chars.get(i).copied()
    }
    pub(crate) fn starts(&self, i: usize, word: &str, fold: bool) -> bool {
        let src = if fold { &self.lower[..] } else { self.chars };
        word.chars().enumerate().all(|(k, w)| src.get(i + k) == Some(&if fold { w.to_ascii_lowercase() } else { w }))
    }
    /// 从 i 起满足条件的字符数，最多数到 cap
    pub(crate) fn run(&self, i: usize, cap: usize, f: impl Fn(char) -> bool) -> usize {
        self.chars.get(i..).map(|s| s.iter().take(cap).take_while(|c| f(**c)).count()).unwrap_or(0)
    }
    pub(crate) fn skip(&self, mut i: usize, f: impl Fn(char) -> bool) -> usize {
        let start = i;
        while i < self.len() && i - start < 16 && f(self.chars[i]) {
            i += 1;
        }
        i
    }
    /// 前一个词（跳过空白）是否为 `word`（不区分大小写）
    pub(crate) fn preceded_by(&self, i: usize, word: &str) -> bool {
        let mut j = i;
        let mut spaces = 0;
        while j > 0 && self.chars[j - 1].is_whitespace() && spaces < 8 {
            j -= 1;
            spaces += 1;
        }
        let n = word.chars().count();
        spaces > 0 && j >= n && self.starts(j - n, word, true) && (j == n || !self.chars[j - n - 1].is_ascii_alphanumeric())
    }
}

/// 键名之后的值：返回 (遮蔽起点, 值起点, 值终点)。支持
/// `KEY=值`、`key: 值`、`"key": "值"`、转义 JSON `\"key\":\"值\"`、`'key' => '值'`、`["KEY"] = "值"`、`("KEY", "值")`、
/// `<apiKey>值</apiKey>`、`--api-key 值`、`setx KEY 值`；引号里的值取到配对引号为止（可含空格、`(`、`;`）
pub(crate) fn find_value(s: &Scan, ks: usize, ke: usize, kind: KeyKind) -> Option<(usize, usize, usize)> {
    let quoted_key = ks > 0 && is_quote(s.chars[ks - 1]);
    // XML 元素：<apiKey>值</apiKey>
    if ks > 0 && s.chars[ks - 1] == '<' && s.at(ke) == Some('>') {
        let vs = ke + 1;
        let ve = vs + s.run(vs, VALUE_MAX, |c| c != '<');
        return Some((vs, vs, ve));
    }
    let mut j = s.skip(ke, |c| is_quote(c) || c == '\\');
    let after_key = j;
    j = s.skip(j, |c| c == ' ' || c == '\t');
    let mut separated = false;
    match s.at(j) {
        Some('=' | ':') => {
            j += 1;
            if matches!(s.at(j), Some('=' | '>' | ':')) {
                j += 1;
            }
            separated = true;
        }
        Some(']') if quoted_key => {
            j = s.skip(j + 1, |c| c == ' ' || c == '\t');
            if s.at(j) != Some('=') {
                return None;
            }
            j += 1;
            separated = true;
        }
        // `("KEY", "值")` 只认函数调用的第一个参数，避免把 JSON 里相邻的下一个字符串当成值（`"token_type": "Bearer", "expires_in"`）
        Some(',') if quoted_key && opens_call(s, ks - 1) => {
            j = s.skip(j + 1, |c| c == ' ' || c == '\t');
            if !s.at(j).is_some_and(is_quote) {
                return None;
            }
            separated = true;
        }
        _ => {}
    }
    let mut bare_password = false;
    if !separated {
        // 空白分隔：`--api-key 值`、`setx KEY 值`、`aws configure set KEY 值`；
        // `.netrc` 的 `password 值`（裸词 password / passwd / pass，值须含数字或符号，避免把散文里的下一个词当成密码）
        let flag = s.chars[ks] == '-';
        let bare = kind == KeyKind::Password
            && !quoted_key
            && matches!(s.lower[ks..ke].iter().collect::<String>().as_str(), "password" | "passwd" | "pass");
        if j == after_key || !(flag || bare || s.preceded_by(ks, "setx") || s.preceded_by(ks, "set")) || s.at(j).is_none_or(|c| c == '-' || c == '\n') {
            return None;
        }
        bare_password = bare && !flag;
    }
    j = s.skip(j, |c| c == ' ' || c == '\t');
    // 值的引号：普通引号或转义引号 `\"`
    let (quote, escaped) = match (s.at(j), s.at(j + 1)) {
        (Some('\\'), Some(q)) if is_quote(q) => (Some(q), true),
        (Some(q), _) if is_quote(q) => (Some(q), false),
        _ => (None, false),
    };
    if quote.is_some() {
        j += if escaped { 2 } else { 1 };
    }
    let mask_from = j;
    if let Some(scheme) = SCHEMES.iter().find(|w| s.starts(j, w, true)) {
        j += scheme.len();
    }
    let vs = j;
    let ve = match (kind, quote) {
        (_, Some(q)) => {
            let n = s.run(vs, VALUE_MAX, |c| c != q && c != '\n');
            // 转义引号的值以 `\"` 结束
            if escaped && n > 0 && s.at(vs + n - 1) == Some('\\') { vs + n - 1 } else { vs + n }
        }
        (KeyKind::Cookie, None) => vs + s.run(vs, VALUE_MAX, |c| c != '\n' && !is_quote(c)),
        (_, None) => {
            let n = s.run(vs, VALUE_MAX, value_char);
            // 后面紧跟 `(` 的是函数调用（`token = get_token()`）
            if s.at(vs + n) == Some('(') {
                return None;
            }
            vs + n
        }
    };
    if bare_password && !s.chars[vs..ve].iter().any(|c| !c.is_alphabetic()) {
        return None;
    }
    Some((mask_from, vs, ve))
}

/// 引号键名前（跳过空白）是不是 `(`：`SetEnvironmentVariable("KEY", "值")`
pub(crate) fn opens_call(s: &Scan, quote: usize) -> bool {
    let mut j = quote;
    let mut spaces = 0;
    while j > 0 && s.chars[j - 1].is_whitespace() && spaces < 8 {
        j -= 1;
        spaces += 1;
    }
    j > 0 && s.chars[j - 1] == '('
}

/// `<add key="ApiKey" value="…"/>` 与 `name="password" value="…"`：属性值本身是凭证键名时，遮后面的 value 属性
pub(crate) fn attr_value(s: &Scan, ke: usize) -> Option<(usize, usize, KeyKind)> {
    let mut j = s.skip(ke, |c| c == ' ');
    if s.at(j) != Some('=') {
        return None;
    }
    j = s.skip(j + 1, |c| c == ' ');
    let q = s.at(j).filter(|c| is_quote(*c))?;
    let ns = j + 1;
    let ne = ns + s.run(ns, IDENT_MAX, |c| c != q);
    let kind = key_kind(&s.chars[ns..ne].iter().collect::<String>())?;
    j = s.skip(ne + 1, |c| c.is_whitespace());
    if !s.starts(j, "value", true) {
        return None;
    }
    j = s.skip(j + 5, |c| c == ' ');
    if s.at(j) != Some('=') {
        return None;
    }
    j = s.skip(j + 1, |c| c == ' ');
    let q = s.at(j).filter(|c| is_quote(*c))?;
    let vs = j + 1;
    let ve = vs + s.run(vs, VALUE_MAX, |c| c != q);
    Some((vs, ve, kind))
}

/// 键值：逐个最长标识符看一次，命中后跳过整个值
pub(crate) fn kv_spans(s: &Scan, spans: &mut Vec<(usize, usize)>) {
    let n = s.len();
    let mut i = 0;
    while i < n {
        if !ident_char(s.chars[i]) {
            i += 1;
            continue;
        }
        let ks = i;
        let mut ke = i;
        while ke < n && ident_char(s.chars[ke]) {
            ke += 1;
        }
        i = ke;
        if ke - ks > IDENT_MAX {
            continue;
        }
        let ident: String = s.chars[ks..ke].iter().collect();
        if let Some(kind) = key_kind(&ident) {
            if let Some((from, vs, ve)) = find_value(s, ks, ke, kind) {
                if should_mask(kind, &s.chars[vs..ve], vs > from) {
                    spans.push((from, ve));
                    i = i.max(ve);
                    continue;
                }
                // 不遮也跳过这个值，避免对同一段反复向前查看（`token=***` 重复时退化）；
                // 再往下看属性写法（`<add key="ApiKey" value="…"/>` 里 key 本身也是凭证键名）
                i = i.max(ve);
            }
        }
        if matches!(ident.to_ascii_lowercase().as_str(), "key" | "name") {
            if let Some((vs, ve, kind)) = attr_value(s, ke).or_else(|| yaml_env_value(s, ke)) {
                if should_mask(kind, &s.chars[vs..ve], false) {
                    spans.push((vs, ve));
                }
                i = i.max(ve);
            }
        }
    }
}

/// k8s / GitHub Actions 的环境变量项：`- name: DB_PASSWORD` 下一行 `value: "…"`
pub(crate) fn yaml_env_value(s: &Scan, ke: usize) -> Option<(usize, usize, KeyKind)> {
    let mut j = s.skip(ke, |c| c == ' ');
    if s.at(j) != Some(':') {
        return None;
    }
    j = s.skip(j + 1, |c| c == ' ');
    let quote = s.at(j).filter(|c| is_quote(*c));
    if quote.is_some() {
        j += 1;
    }
    let ns = j;
    let ne = ns + s.run(ns, IDENT_MAX, ident_char);
    let kind = key_kind(&s.chars[ns..ne].iter().collect::<String>())?;
    // 只看紧接着的下一行：缩进 + 可选的 `- ` + `value:`
    let mut j = ne + s.run(ne, IDENT_MAX, |c| c != '\n');
    if s.at(j) != Some('\n') {
        return None;
    }
    j = s.skip(j + 1, |c| c == ' ' || c == '\t' || c == '-');
    if !s.starts(j, "value", true) {
        return None;
    }
    j = s.skip(j + 5, |c| c == ' ');
    if s.at(j) != Some(':') {
        return None;
    }
    j = s.skip(j + 1, |c| c == ' ');
    let (vs, ve) = match s.at(j).filter(|c| is_quote(*c)) {
        Some(q) => (j + 1, j + 1 + s.run(j + 1, VALUE_MAX, |c| c != q && c != '\n')),
        None => (j, j + s.run(j, VALUE_MAX, |c| c != '\n' && !c.is_whitespace())),
    };
    Some((vs, ve, kind))
}

/// 前缀 Key 与 JWT：只在词首判断，看过的值整段跳过
pub(crate) fn token_spans(s: &Scan, spans: &mut Vec<(usize, usize)>) {
    let n = s.len();
    let mut i = 0;
    while i < n {
        let boundary = i == 0 || !(s.chars[i - 1].is_ascii_alphanumeric() || s.chars[i - 1] == '_');
        if !boundary || !s.chars[i].is_ascii_alphanumeric() {
            i += 1;
            continue;
        }
        if s.starts(i, "eyJ", false) {
            let a = s.run(i, VALUE_MAX, jwt_char);
            if a >= 13 && s.at(i + a) == Some('.') {
                let b = s.run(i + a + 1, VALUE_MAX, jwt_char);
                if b >= 10 && s.at(i + a + 1 + b) == Some('.') {
                    let c = s.run(i + a + b + 2, VALUE_MAX, jwt_char);
                    spans.push((i + 3, i + a + b + 2 + c));
                    i += a + b + 2 + c;
                    continue;
                }
            }
            i += a.max(1);
            continue;
        }
        if let Some(prefix) = PREFIX_KEYS.iter().find(|p| s.starts(i, p, false)) {
            let len = prefix.chars().count();
            let value = s.run(i + len, VALUE_MAX, |c| c.is_ascii_alphanumeric() || matches!(c, '-' | '_' | '.'));
            if value >= PREFIX_MIN {
                spans.push((i + len, i + len + value));
            }
            i += len + value;
            continue;
        }
        if s.starts(i, "bearer ", true) {
            let value = s.run(i + 7, VALUE_MAX, value_char);
            if value >= 16 {
                spans.push((i + 7, i + 7 + value));
            }
            i += 7 + value;
            continue;
        }
        i += s.run(i, VALUE_MAX, |c| c.is_ascii_alphanumeric() || c == '_').max(1);
    }
}

/// 长串兜底：每个最长串只判断一次
pub(crate) fn long_spans(s: &Scan, spans: &mut Vec<(usize, usize)>) {
    let n = s.len();
    let mut i = 0;
    while i < n {
        if !long_char(s.chars[i]) {
            i += 1;
            continue;
        }
        let start = i;
        while i < n && long_char(s.chars[i]) {
            i += 1;
        }
        while i < n && s.chars[i] == '=' && i - start < VALUE_MAX {
            i += 1;
        }
        let token = &s.chars[start..i];
        let mut longest = 0;
        let mut cur = 0;
        for c in token {
            cur = if c.is_ascii_alphanumeric() { cur + 1 } else { 0 };
            longest = longest.max(cur);
        }
        if token.len() < LONG_MIN || longest < LONG_SEGMENT_MIN {
            continue;
        }
        let body: Vec<char> = token.iter().copied().filter(|c| *c != '=').collect();
        let hex = body.iter().all(|c| c.is_ascii_hexdigit());
        let count = |f: fn(&char) -> bool| body.iter().filter(|c| f(c)).count();
        let mixed = count(char::is_ascii_uppercase) >= 3 && count(char::is_ascii_lowercase) >= 3 && count(char::is_ascii_digit) >= 3;
        // `commit <40 位 hex>`：git 提交号放行
        let commit = hex && body.len() == 40 && start >= 7 && s.starts(start - 7, "commit ", true);
        if !(hex || mixed) || commit {
            continue;
        }
        let mut end = i;
        // 智谱：32 位 hex 后接 `.` 与 ≥ 8 位字母数字
        if hex && s.at(end) == Some('.') {
            let tail = s.run(end + 1, VALUE_MAX, |c| c.is_ascii_alphanumeric());
            if tail >= 8 {
                end += 1 + tail;
                i = end;
            }
        }
        spans.push((start, end));
    }
}

/// URL 里的账号密码（`scheme://user:pass@host`）：只遮密码，保留用户名与主机
pub(crate) fn url_spans(s: &Scan, spans: &mut Vec<(usize, usize)>) {
    let n = s.len();
    let mut i = 0;
    while i + 3 <= n {
        if !s.starts(i, "://", false) {
            i += 1;
            continue;
        }
        let us = i + 3;
        let len = s.run(us, 256, |c| !c.is_whitespace() && !matches!(c, '/' | '@' | '?' | '#' | '"' | '\''));
        i = us + len;
        if s.at(i) != Some('@') {
            continue;
        }
        if let Some(colon) = s.chars[us..i].iter().position(|c| *c == ':') {
            if i - (us + colon + 1) >= 1 {
                spans.push((us + colon + 1, i));
            }
        }
    }
}

/// 一行里到当前词为止出现过的命令（只看开关之前的词：`docker run -p 3306:3306 mysql` 里的 mysql 是镜像名）
#[derive(Default)]
pub(crate) struct SeenTools {
    pub(crate) mysql: bool,
    pub(crate) p_flag: bool,
    pub(crate) docker: bool,
    pub(crate) login: bool,
    pub(crate) redis: bool,
    pub(crate) sqlcmd: bool,
    pub(crate) keygen: bool,
    pub(crate) keytool: bool,
    pub(crate) seven_zip: bool,
}

impl SeenTools {
    pub(crate) fn note(&mut self, name: &str) {
        match name {
            "mysql" | "mysqldump" | "mysqladmin" | "mariadb" => self.mysql = true,
            "sshpass" | "mongosh" | "mongo" => self.p_flag = true,
            "docker" | "podman" => self.docker = true,
            "login" => self.login = true,
            "redis-cli" => self.redis = true,
            "sqlcmd" => self.sqlcmd = true,
            "ssh-keygen" => self.keygen = true,
            "keytool" => self.keytool = true,
            "7z" | "7za" => self.seven_zip = true,
            _ => {}
        }
    }
    /// 这个词是不是「下一个词是密码」的开关
    pub(crate) fn takes_password(&self, w: &str) -> bool {
        match w {
            "-p" => self.mysql || self.p_flag || (self.docker && self.login),
            "-a" => self.redis,
            "-P" => self.sqlcmd || self.keygen,
            "-N" => self.keygen,
            "-storepass" | "-keypass" | "-srcstorepass" | "-deststorepass" => self.keytool,
            _ => false,
        }
    }
    /// `-p密码` 连写
    pub(crate) fn attached(&self) -> bool {
        self.mysql || self.seven_zip
    }
}

/// 命令行里的账号密码：`curl -u user:pass` / `--user user:pass`；按同一行的命令识别密码开关
/// （mysql `-p密码` / `-p 密码`、`docker login -p`、`sshpass -p`、`mongosh -p`、`redis-cli -a`、`sqlcmd -P`、
/// `ssh-keygen -N`、`keytool -storepass`、`7z -p密码`）。`docker run -p 3306:3306` 不受影响（没有 login）
pub(crate) fn flag_spans(s: &Scan, spans: &mut Vec<(usize, usize)>) {
    let n = s.len();
    let mut words: Vec<(usize, usize)> = Vec::new();
    let mut names: Vec<String> = Vec::new();
    let mut i = 0;
    let flush = |words: &mut Vec<(usize, usize)>, names: &mut Vec<String>, spans: &mut Vec<(usize, usize)>| {
        let mut seen = SeenTools::default();
        for (k, &(ws, we)) in words.iter().enumerate() {
            let w: String = s.chars[ws..we].iter().collect();
            let next = words.get(k + 1).copied();
            let takes_password = seen.takes_password(&w);
            let attached = seen.attached();
            seen.note(&names[k]);
            if w == "-u" || w == "--user" {
                if let Some((ns, ne)) = next {
                    if let Some(colon) = s.chars[ns..ne].iter().position(|c| *c == ':') {
                        if ne > ns + colon + 1 {
                            spans.push((ns + colon + 1, ne));
                        }
                    }
                }
            } else if let Some(rest) = w.strip_prefix("--user=") {
                if let Some(colon) = rest.find(':') {
                    let at = ws + "--user=".len() + rest[..colon].chars().count() + 1;
                    if we > at {
                        spans.push((at, we));
                    }
                }
            } else if takes_password {
                // 独立开关：下一个词是密码（`mysql -p appdb` 这种「下一个词是库名」无法区分，宁可多遮）
                if let Some((ns, ne)) = next.filter(|(ns, _)| s.chars[*ns] != '-') {
                    let (ns, ne) = strip_quotes(s, ns, ne);
                    if ne > ns {
                        spans.push((ns, ne));
                    }
                }
            } else if attached && w.starts_with("-p") && !w.starts_with("--") && w.len() > 2 {
                spans.push((ws + 2, we));
            }
        }
        words.clear();
        names.clear();
    };
    while i < n {
        let c = s.chars[i];
        if c == '\n' {
            flush(&mut words, &mut names, spans);
            i += 1;
            continue;
        }
        if c.is_whitespace() {
            i += 1;
            continue;
        }
        let ws = i;
        while i < n && !s.chars[i].is_whitespace() {
            i += 1;
        }
        let word: String = s.lower[ws..i.min(ws + 24)].iter().collect();
        let base = word.trim_end_matches(".exe");
        names.push(base.rsplit(['/', '\\']).next().unwrap_or(base).to_string());
        words.push((ws, i));
    }
    flush(&mut words, &mut names, spans);
}

/// 私钥块最多遮这么多字（找不到结束行时也只遮到这里）
pub(crate) const PEM_MAX: usize = 8_192;
pub(crate) const CN_LABELS: [&str; 6] = ["密码", "密钥", "秘钥", "令牌", "口令", "私钥"];

/// 其余写法：
/// - `-----BEGIN … PRIVATE KEY-----` 到 `-----END` 之间整段遮（逐行判断长串会漏掉短行）；
/// - 中文标签 `密码:` / `密钥：` 后的值（用户原话原样进事实区）；
/// - PowerShell `ConvertTo-SecureString "值" -AsPlainText`（也认 `-String '值'`）
pub(crate) fn misc_spans(s: &Scan, spans: &mut Vec<(usize, usize)>) {
    let n = s.len();
    let mut i = 0;
    while i < n {
        if s.starts(i, "-----BEGIN ", false) {
            let header = s.run(i, 128, |c| c != '\n');
            let line: String = s.chars[i..i + header].iter().collect();
            if line.contains("PRIVATE KEY") {
                let body = i + header;
                let mut j = body;
                while j < n && j - body < PEM_MAX && !s.starts(j, "-----END", false) {
                    j += 1;
                }
                spans.push((body, j));
                i = j.max(i + 1);
                continue;
            }
            i += header.max(1);
            continue;
        }
        if let Some(label) = CN_LABELS.iter().find(|w| s.starts(i, w, false)) {
            let mut j = s.skip(i + label.chars().count(), |c| c == ' ');
            if matches!(s.at(j), Some(':' | '：')) {
                j = s.skip(j + 1, |c| c == ' ');
                let quote = s.at(j).filter(|c| is_quote(*c));
                let vs = j + usize::from(quote.is_some());
                let len = match quote {
                    Some(q) => s.run(vs, VALUE_MAX, |c| c != q && c != '\n'),
                    None => s.run(vs, VALUE_MAX, |c| value_char(c) && !matches!(c, '，' | '。' | '；' | '、' | '）')),
                };
                let value = &s.chars[vs..vs + len];
                // 值至少 6 位且含 ASCII 字母或数字（中文散文「密码：错误」不遮）
                if len >= PASSWORD_MIN && value.iter().any(|c| c.is_ascii_alphanumeric()) {
                    spans.push((vs, vs + len));
                }
                i = vs + len.max(1);
                continue;
            }
            i += 1;
            continue;
        }
        if s.starts(i, "convertto-securestring", true) {
            let mut j = s.skip(i + 22, |c| c == ' ');
            if s.starts(j, "-string", true) {
                j = s.skip(j + 7, |c| c == ' ');
            }
            if let Some(q) = s.at(j).filter(|c| is_quote(*c)) {
                let vs = j + 1;
                let ve = vs + s.run(vs, VALUE_MAX, |c| c != q && c != '\n');
                if ve > vs {
                    spans.push((vs, ve));
                }
                i = ve.max(i + 1);
                continue;
            }
            i += 22;
            continue;
        }
        i += 1;
    }
}

/// 去掉词两端的配对引号（`-p 'pw'` → `pw`）
pub(crate) fn strip_quotes(s: &Scan, ws: usize, we: usize) -> (usize, usize) {
    match (s.at(ws), we.checked_sub(1).and_then(|e| s.at(e))) {
        (Some(a), Some(b)) if we - ws >= 2 && is_quote(a) && a == b => (ws + 1, we - 1),
        _ => (ws, we),
    }
}

/// 遮蔽会话来源文字里的凭证（事实区的用户原话、命令、验证输出、报错，发给模型的原文、原工具摘要，最终文档的叙述部分）：
/// - 凭证键名（`api_key|token|secret|credential|auth|authoriz*|password|passwd|pass|pwd|…_key|cookie`）后的值，
///   覆盖 `=`、`:`、JSON（含转义）、`=>`、`["KEY"] =`、`("KEY", "值")`、XML、`--flag 值`、`setx KEY 值` 等写法；
/// - 前缀 Key（`sk-`、`ghp_`、`gsk_`、`npm_`、`ya29.`、`AKIA`、`xox?-` 等）、JWT、`Bearer …`；
/// - URL 与 `curl -u`、mysql `-p` 里的密码；
/// - ≥ 32 位且含 ≥ 20 位连续字母数字段的 hex / base64 长串（`commit <sha>` 放行）。
///
/// 程序生成的内容（Git 快照、文件清单、回查路径）不经过这里。结果可重复处理
pub fn redact(text: &str) -> String {
    let chars: Vec<char> = text.chars().collect();
    let scan = Scan { chars: &chars, lower: chars.iter().map(|c| c.to_ascii_lowercase()).collect() };
    let mut spans: Vec<(usize, usize)> = Vec::new();
    kv_spans(&scan, &mut spans);
    token_spans(&scan, &mut spans);
    long_spans(&scan, &mut spans);
    url_spans(&scan, &mut spans);
    flag_spans(&scan, &mut spans);
    misc_spans(&scan, &mut spans);
    // 与网关隐私脱敏共用的识别规则（3.7）：本模块没有的卡号、身份证号（校验位合法才算）
    // 取网关引擎的结果；邮箱按本模块的约定不遮（通知与交接里它是联系人上下文）。
    // 设置里「从不脱敏」的值两边都放过
    let offsets: Vec<usize> = text.char_indices().map(|(i, _)| i).collect();
    let to_char = |byte: usize| offsets.partition_point(|&b| b < byte);
    for (a, b) in crate::infra::redact::card_and_idcard_spans(text) {
        spans.push((to_char(a), to_char(b)));
    }
    spans.retain(|(a, b)| b > a && !crate::infra::redact::allowed_value(&chars[*a..*b].iter().collect::<String>()));
    if spans.is_empty() {
        return text.to_string();
    }
    spans.sort_unstable();
    let mut out = String::with_capacity(text.len());
    let mut cursor = 0;
    for (a, b) in spans {
        if b <= cursor {
            continue;
        }
        let a = a.max(cursor);
        out.extend(&chars[cursor..a]);
        if !out.ends_with(MASK) || a > cursor {
            out.push_str(MASK);
        }
        cursor = b;
    }
    out.extend(&chars[cursor..]);
    out
}
