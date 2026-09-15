//! `~/.ssh/config` 的**受限子集**导入（plan 0506 / ADR-0003 D14）。
//!
//! 三件事在这里定死，其余都是它们的后果 —— 细节与实测依据见 plan 0506：
//!
//! 1. **只导入六条**：`Host` / `HostName` / `User` / `Port` / `IdentityFile` / `ProxyJump`。
//! 2. **求值只有一条规则**：每个参数**首次取到的值生效**（`ssh_config(5)`：*the first
//!    specified value will be used*）。实测（系统 OpenSSH 10.5 的 `ssh -G`）：文件开头
//!    `Host *` + `Port 2222`、后面 `Host foo` + `Port 33`，`foo` 的端口是 **2222** ——
//!    所以"一个 `Host` 块 = 一行"那种读法会**静默连错端口**。另两条实测：文件开头到第一个
//!    `Host` / `Match` 之间的指令是**全局**的（等价于 `Host *`）；**关键字**不分大小写，
//!    而 **`Host` 模式**分大小写（`Host foo` 不匹配 `FOO`）。通配块本身**不是条目**，
//!    它的取值靠匹配并入具体条目（`Host *` 就是这么用的）。
//! 3. **看不懂就报错，不猜**（D14）。三档：
//!    * 六条 → 导入；
//!    * 其余 OpenSSH 关键字（保活、算法、认证方式、known_hosts 策略、转发、日志……）→
//!      **逐条警告**并继续：它们不改"连到哪台机器"，也不改"我们信任哪把主机密钥"，
//!      而"没生效"会逐条出现在报告里；
//!    * `Match` / `Include` / 会改目的地或信任来源的那些 / `IgnoreUnknown` / **表里没有的**
//!      → **整份报错**（一次列全，带行号）。
//!    ⚠️ 最后一档的兜底是**默认报错**：真有一条我们漏判的关键字，它会落在"表里没有的"
//!    那一档，方向是保守的那一边。
//!
//! ## 为什么是**纯函数**
//!
//! [`parse`] 不读盘、不读环境、不碰库：`~/.ssh/config` 在哪、本机用户叫什么，都是调用方的事。
//! 于是整套语义（首次取值、通配与取反、三档分类、`ProxyJump` 的三种写法）可以在**没有 app、
//! 没有库、没有网络**的地方被钉死。
//!
//! ## 两处照实记的边界
//!
//! * **私钥不导入**：`IdentityFile` 只让条目落成 `publickey`（`key_id` 留空 = 走 ssh-agent），
//!   并在报告里逐条说清"这个密钥文件没有导入"（把私钥复制进库是另一件事，见 plan 的「非目标」）。
//! * **`ProxyJump` 只收裸名字**：`[user@]host[:port]` 与 ssh URI 这种写法**报错**并给出改法
//!   （写一条 `Host` 块再用别名引用）。原因：池里一跳就是**一行**，`user@` / `:port` 是"对
//!   这一跳的局部改写"，库里没有对应的表示 —— 硬塞会变成一条用户没写过的行。

use std::collections::{BTreeMap, BTreeSet, VecDeque};

/// 跳板链的深度上限（与 `pools` 的 `MAX_JUMP_DEPTH` 同一个数、同一个理由：兜底防死循环）。
const MAX_HOPS: usize = 32;

/// 一条要导入的条目（**还没有 id**，跳板也还只是一个名字）。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Target {
    /// 池里的 `name`：`Host` 模式的原样文本（它是**字面**模式，不带通配）。
    pub name: String,
    /// 要连的主机名（`HostName`；没写就是名字本身）。
    pub host: String,
    pub port: u16,
    pub user: String,
    /// `IdentityFile` 的原样文本（**只用来出报告**：私钥不导入）。
    pub key_file: Option<String>,
    /// 跳板：**这次导入里另一个 [`Target::name`]**（`None` = 直连）。
    pub jump: Option<String>,
    /// 这条是**为跳板补建**的（配置里没有它的 `Host` 块）。
    ///
    /// 池里已有同名行时它**不覆盖**、只沿用 —— 补建是"够用就好"的兜底，不是用户的声明。
    pub provisional: bool,
}

/// 解析出来的一批条目 + 两份记录（没生效的指令、结构性说明）。
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct Imported {
    pub targets: Vec<Target>,
    /// 没生效的指令（三档里的第二档 + `IdentityFile`），逐条带行号。
    pub ignored: Vec<Finding>,
    /// 结构性说明：通配块未成条目、为跳板补建了行、文件里没有条目……
    pub notes: Vec<String>,
}

/// 一条**给人看的**记录：行号 + 关键字 + 说法。
///
/// `keyword` 是**小写规范形**（关键字不分大小写，`HOST` 与 `host` 是同一条）——
/// 机器断言靠它，解释靠 `message`。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Finding {
    pub line: usize,
    pub keyword: String,
    pub message: String,
}

/// 把一份 `~/.ssh/config` 的文本读成一批条目（`default_user` = 没写 `User` 时用它）。
///
/// 失败 = **整份不导入**，错误里是**全部**问题（一次列全，用户一趟改完）。静默跳过不存在：
/// 每一条看不懂的指令都必须在 `Err` 或 [`Imported::ignored`] 里露面。
pub fn parse(text: &str, default_user: &str) -> Result<Imported, Vec<Finding>> {
    let mut scan = Scan::default();
    scan.run(text);
    if !scan.problems.is_empty() {
        return Err(scan.problems);
    }
    scan.evaluate(default_user)
}

// ── 词法：一行 → (关键字, 值) ────────────────────────────────────────────────

/// 去掉注释、切出标记、把关键字小写化。
///
/// 三条都属于 OpenSSH 的文本约定：`#` 在**引用之外**就可以开始注释；值可以用双引号包住
/// （好让里面能有空格）；关键字与值之间是空白**或者恰好一个 `=`**（`Port=22` / `Port = 22`）。
/// 关键字**不分大小写**，值分 —— 所以只小写关键字。
fn split(line: &str) -> Option<(String, Vec<String>)> {
    let mut tokens = tokens(line);
    if tokens.is_empty() {
        return None;
    }
    let mut keyword = tokens.remove(0);
    // `Host=foo`：`=` 粘在关键字上
    if let Some(at) = keyword.find('=') {
        let value = keyword[at + 1..].to_owned();
        keyword.truncate(at);
        if !value.is_empty() {
            tokens.insert(0, value);
        }
    } else if let Some(first) = tokens.first() {
        // `Host = foo`：`=` 自己是一个标记
        if first == "=" {
            tokens.remove(0);
        } else if let Some(rest) = first.strip_prefix('=') {
            let rest = rest.to_owned();
            tokens[0] = rest;
            if tokens[0].is_empty() {
                tokens.remove(0);
            }
        }
    }
    Some((keyword.to_ascii_lowercase(), tokens))
}

/// 按空白切标记，认双引号与引用内的 `\` 转义，引用外的 `#` 截断本行。
fn tokens(line: &str) -> Vec<String> {
    let mut out = Vec::new();
    let mut current = String::new();
    let mut quoted = false;
    let mut escaped = false;
    let mut started = false;
    for ch in line.chars() {
        if escaped {
            current.push(ch);
            escaped = false;
            continue;
        }
        match ch {
            '\\' if quoted => escaped = true,
            '"' => {
                quoted = !quoted;
                started = true;
            }
            '#' if !quoted => break,
            c if c.is_whitespace() && !quoted => {
                if started || !current.is_empty() {
                    out.push(std::mem::take(&mut current));
                }
                started = false;
            }
            c => {
                current.push(c);
                started = true;
            }
        }
    }
    if started || !current.is_empty() {
        out.push(current);
    }
    out
}

// ── 块与取值 ────────────────────────────────────────────────────────────────

/// `Host` 行上的一个模式（可能带 `!` 取反、`*` / `?` 通配）。
#[derive(Debug, Clone, PartialEq, Eq)]
struct Pattern {
    text: String,
    negated: bool,
}

/// 一个配置段：`Host` 行（或文件开头的**全局段**）到下一个 `Host` / `Match` 之前。
#[derive(Debug, Clone, Default)]
struct Block {
    /// 这一段的 `Host` 模式。空 = 匹配不了任何名字（`Match` 段 —— 有它就是报错，不会走到求值）。
    patterns: Vec<Pattern>,
    values: Values,
}

/// 六条里那些**带值**的（`Host` 是分段指令，不进这里）。只留首次取到的那个。
#[derive(Debug, Clone, Default, PartialEq, Eq)]
struct Values {
    host_name: Option<String>,
    user: Option<String>,
    port: Option<u16>,
    /// `IdentityFile` 在 OpenSSH 里是**累加**的列表，我们只要"写过没有 + 第一个"。
    identity: Option<String>,
    /// `ProxyJump` 的原文 + 它的行号（报错要指得到那一行）。
    jump: Option<(usize, String)>,
}

impl Values {
    /// 只填**还空着**的槽（"首次取到的值生效"就落在这三行里）。
    fn fill_from(&mut self, other: &Self) {
        if self.host_name.is_none() {
            self.host_name.clone_from(&other.host_name);
        }
        if self.user.is_none() {
            self.user.clone_from(&other.user);
        }
        if self.port.is_none() {
            self.port = other.port;
        }
        if self.identity.is_none() {
            self.identity.clone_from(&other.identity);
        }
        if self.jump.is_none() {
            self.jump.clone_from(&other.jump);
        }
    }
}

/// 扫一遍文本：分段、分类、把能当场判的错记下来。
#[derive(Debug, Default)]
struct Scan {
    blocks: Vec<Block>,
    /// 文件里出现过的**字面**模式（每个都会变成一个条目）。
    literals: Vec<String>,
    problems: Vec<Finding>,
    ignored: Vec<Finding>,
    notes: Vec<String>,
    /// 当前段的下标（0 = 全局段）。
    current: usize,
}

impl Scan {
    fn run(&mut self, text: &str) {
        // 下标 0 是**文件开头到第一个 `Host` / `Match` 之间**的全局段：按 OpenSSH 的语义，
        // 它等价于 `Host *`（实测）。
        self.blocks.push(Block {
            patterns: vec![Pattern {
                text: "*".to_owned(),
                negated: false,
            }],
            values: Values::default(),
        });

        for (index, raw) in text.lines().enumerate() {
            let line = index + 1;
            let Some((keyword, values)) = split(raw) else {
                continue;
            };

            if keyword == "host" {
                self.open_block(&values, line);
                continue;
            }
            if keyword == "match" {
                self.problems.push(finding(
                    line,
                    &keyword,
                    "条件块无法求值，而它可以给**任何**条目加设定（`Match host` 比的还是 `HostName` \
                     替换之后的名字）—— 宁可整份不导入，也不猜哪些条目受影响",
                ));
                // 之后的指令落进一个匹配不了任何名字的段（反正有 `Match` 就已经整份报错了）。
                self.blocks.push(Block::default());
                self.current = self.blocks.len() - 1;
                continue;
            }

            match classify(&keyword) {
                Class::Supported => self.assign(&keyword, &values, line),
                Class::Ignored(category) => {
                    self.ignored
                        .push(finding(line, &keyword, category.message()))
                }
                Class::Refused(why) => self.problems.push(finding(line, &keyword, why)),
            }
        }
    }

    fn open_block(&mut self, values: &[String], line: usize) {
        if values.is_empty() {
            self.problems
                .push(finding(line, "Host", "这条指令后面没有模式"));
            return;
        }
        let patterns: Vec<Pattern> = values
            .iter()
            .map(|value| match value.strip_prefix('!') {
                Some(rest) => Pattern {
                    text: rest.to_owned(),
                    negated: true,
                },
                None => Pattern {
                    text: value.clone(),
                    negated: false,
                },
            })
            .collect();

        if patterns.iter().all(|pattern| pattern.negated) {
            self.notes.push(format!(
                "第 {line} 行：`Host {}` 只有取反模式，不产生条目",
                values.join(" ")
            ));
        } else if patterns
            .iter()
            .any(|pattern| !pattern.negated && !is_literal(&pattern.text))
        {
            self.notes.push(format!(
                "第 {line} 行：`Host {}` 是通配块，不产生条目（它的取值已并入匹配到的条目）",
                values.join(" ")
            ));
        }

        for pattern in &patterns {
            let literal = !pattern.negated && is_literal(&pattern.text);
            if literal && !self.literals.contains(&pattern.text) {
                self.literals.push(pattern.text.clone());
            }
        }

        self.blocks.push(Block {
            patterns,
            values: Values::default(),
        });
        self.current = self.blocks.len() - 1;
    }

    /// 六条里带值的那五条：**首次取到的值生效**（后面的同名列安静地不生效 —— 那是 OpenSSH
    /// 的语义，不是问题）。写法本身不合法（值的个数、端口范围）则**一律报错**：
    /// 一行看不懂的指令可能是全局段里的，影响面不限于某台主机。
    fn assign(&mut self, keyword: &str, values: &[String], line: usize) {
        if values.len() != 1 {
            self.problems.push(finding(
                line,
                keyword,
                &format!("这条指令只接受一个值，这一行有 {} 个", values.len()),
            ));
            return;
        }
        let value = values[0].clone();
        let slot = &mut self.blocks[self.current].values;

        match keyword {
            "hostname" => {
                slot.host_name.get_or_insert(value);
            }
            "user" => {
                if value.is_empty() {
                    self.problems
                        .push(finding(line, keyword, "这条指令的值是空的"));
                } else {
                    slot.user.get_or_insert(value);
                }
            }
            "port" => match value.parse::<u16>() {
                Ok(port) if port > 0 => {
                    slot.port.get_or_insert(port);
                }
                _ => self.problems.push(finding(
                    line,
                    keyword,
                    &format!("端口得是 1..65535 的数字，这一行写的是 `{value}`"),
                )),
            },
            "identityfile" => {
                // 私钥**不导入**（plan 0506 的非目标）：这句说的是"你写了什么、我们做了什么"，
                // 而不是"这条不生效"—— 它生效了一半（认证方式按 publickey 落库）。
                self.ignored.push(finding(line, keyword, IDENTITY_FILE));
                slot.identity.get_or_insert(value);
            }
            "proxyjump" => {
                slot.jump.get_or_insert((line, value));
            }
            other => unreachable!("`{other}` 不在支持的六条里"),
        }
    }

    /// 求值 → 条目。到这里已经保证**没有**任何 fatal 问题。
    fn evaluate(&mut self, default_user: &str) -> Result<Imported, Vec<Finding>> {
        let mut state = State {
            blocks: std::mem::take(&mut self.blocks),
            entries: BTreeMap::new(),
            notes: std::mem::take(&mut self.notes),
            problems: Vec::new(),
        };

        for name in &self.literals {
            let values = effective(&state.blocks, name);
            state
                .entries
                .insert(name.clone(), Entry::new(values, false));
        }

        state.connect_jumps();

        if !state.problems.is_empty() {
            return Err(state.problems);
        }

        if state.entries.is_empty() {
            state
                .notes
                .push("配置里没有具体的主机条目（只有全局段或通配块），没有东西可导入".to_owned());
        }

        let targets = state
            .entries
            .into_iter()
            .map(|(name, entry)| entry.into_target(name, default_user))
            .collect();
        Ok(Imported {
            targets,
            ignored: std::mem::take(&mut self.ignored),
            notes: state.notes,
        })
    }
}

/// 解析过程中的一张表：名字 → （六条的取值 + 是不是补建的 + 已定的跳板）。
struct State {
    blocks: Vec<Block>,
    entries: BTreeMap<String, Entry>,
    notes: Vec<String>,
    problems: Vec<Finding>,
}

#[derive(Debug, Clone, Default)]
struct Entry {
    values: Values,
    hop: Option<String>,
    provisional: bool,
}

impl Entry {
    fn new(values: Values, provisional: bool) -> Self {
        Self {
            values,
            hop: None,
            provisional,
        }
    }

    fn into_target(self, name: String, default_user: &str) -> Target {
        Target {
            // `HostName` 没写就是名字本身。大小写**按 `ssh -G` 实测**：OpenSSH 把没写
            // `HostName` 时的目标名小写化了（`ssh -G MiXeD` → `hostname mixed`），
            // 而 `Host` **模式**的匹配用的是命令行原样（`Host foo` 不匹配 `FOO`）。
            host: self
                .values
                .host_name
                .unwrap_or_else(|| name.to_ascii_lowercase()),
            port: self.values.port.unwrap_or(22),
            user: self.values.user.unwrap_or_else(|| default_user.to_owned()),
            key_file: self.values.identity,
            jump: self.hop,
            provisional: self.provisional,
            name,
        }
    }
}

impl State {
    /// 补建一条跳板条目（幂等）：配置里没有它的 `Host` 块，但名字出现在某个 `ProxyJump` 里。
    fn ensure(&mut self, name: &str) {
        if self.entries.contains_key(name) {
            return;
        }
        let values = effective(&self.blocks, name);
        self.notes.push(format!(
            "为跳板 `{name}` 补建了一条条目（配置里没有它的 `Host` 块）"
        ));
        self.entries
            .insert(name.to_owned(), Entry::new(values, true));
    }

    /// 挂一跳。**冲突就报错**：同一个名字被两种写法指到不同的跳板上，静默取一个等于编造配置。
    fn set_hop(&mut self, name: &str, hop: &str) {
        let Some(entry) = self.entries.get_mut(name) else {
            return;
        };
        match &entry.hop {
            Some(existing) if existing == hop => {}
            Some(existing) => {
                let line = entry
                    .values
                    .jump
                    .as_ref()
                    .map(|(line, _)| *line)
                    .unwrap_or(0);
                self.problems.push(finding(
                    line,
                    "proxyjump",
                    &format!("`{name}` 被两种写法指到了不同的跳板（`{existing}` 与 `{hop}`）—— 无法同时表达"),
                ));
            }
            None => entry.hop = Some(hop.to_owned()),
        }
    }

    /// 把所有条目的 `ProxyJump` 接起来。
    ///
    /// **不是递归**是有意的：`ProxyJump` 可以点出一个配置里没有的名字，那样我们会**补建**一条
    /// 条目，而补建的那条自己（按匹配到的块求值）也可能有 `ProxyJump` —— 递归写法在最坏情况下
    /// 会随分支指数展开（32 跳 × 每跳 2 个名字），等于让用户自己的配置把 app 卡住。
    /// 这里用一个工作队列：每条条目**只读一次**自己的 `ProxyJump`，补建出来的名字排在队尾。
    fn connect_jumps(&mut self) {
        let mut queue: VecDeque<String> = self.entries.keys().cloned().collect();
        let mut read: BTreeSet<String> = BTreeSet::new();
        while let Some(name) = queue.pop_front() {
            if !read.insert(name.clone()) {
                continue;
            }
            let Some((line, raw)) = self.entries[&name].values.jump.clone() else {
                continue;
            };
            let Some(specs) = self.chain(&raw, line) else {
                continue; // 写法不支持，已经记了问题
            };
            for spec in &specs {
                if !self.entries.contains_key(spec) {
                    self.ensure(spec);
                    queue.push_back(spec.clone());
                }
            }
            // `ProxyJump a,b` = 先连 a 再连 b：于是靠目标最近的是 b，而 b 的跳板是 a。
            for pair in specs.windows(2) {
                self.set_hop(&pair[1], &pair[0]);
            }
            if let Some(nearest) = specs.last() {
                self.set_hop(&name, nearest);
            }
        }
        self.check_cycles();
    }

    /// 成环与过深：**在这里就说清是哪个名字**，别等到库那一侧只回一句"链不对"。
    ///
    /// 库那一侧也挡（`pools::hosts::update_host` 与读路径各有一份），那两份挡的是**绕过
    /// 这段代码**的写入；而这里能给出"从哪个名字起绕回来的"。
    fn check_cycles(&mut self) {
        for start in self.entries.keys().cloned().collect::<Vec<_>>() {
            let mut walked = 0;
            let mut cursor = Some(start.clone());
            while let Some(current) = cursor {
                walked += 1;
                if current == start && walked > 1 {
                    let line = self.entries[&start]
                        .values
                        .jump
                        .as_ref()
                        .map(|(line, _)| *line)
                        .unwrap_or(0);
                    self.problems.push(finding(
                        line,
                        "proxyjump",
                        &format!("跳板链从 `{start}` 出发绕回了它自己（成环），这条配置连不通"),
                    ));
                    break;
                }
                if walked > MAX_HOPS {
                    self.problems.push(finding(
                        0,
                        "proxyjump",
                        &format!("跳板链超过 {MAX_HOPS} 跳"),
                    ));
                    break;
                }
                cursor = self
                    .entries
                    .get(&current)
                    .and_then(|entry| entry.hop.clone());
            }
        }
    }

    /// `ProxyJump` 的值 → 一跳串。**只收裸名字**（理由见模块文档）。
    fn chain(&mut self, raw: &str, line: usize) -> Option<Vec<String>> {
        let raw = raw.trim();
        if raw.eq_ignore_ascii_case("none") {
            return Some(Vec::new());
        }
        let mut specs = Vec::new();
        for spec in raw.split(',') {
            let spec = spec.trim();
            if spec.is_empty() || spec.contains(['@', ':', '/']) {
                self.problems.push(finding(
                    line,
                    "proxyjump",
                    &format!(
                        "`{spec}` 这种写法本程序不支持：请给它写一条 `Host` 块，再用别名引用 —— \
                         池里一跳就是一行，`user@` / `:port` 是对这一跳的局部改写，库里没有对应的表示"
                    ),
                ));
                return None;
            }
            specs.push(spec.to_owned());
        }
        Some(specs)
    }
}

/// 对这个名字把全部段扫一遍，取每个参数的**第一个**值。
fn effective(blocks: &[Block], name: &str) -> Values {
    let mut out = Values::default();
    for block in blocks {
        if block_matches(block, name) {
            out.fill_from(&block.values);
        }
    }
    out
}

/// 这一段对这个名字成立吗：**有**一个非取反模式匹配，且**没有**取反模式匹配
/// （与 OpenSSH 的 `match_pattern_list` 同序：任一取反命中即整段作废）。
fn block_matches(block: &Block, name: &str) -> bool {
    let mut positive = false;
    for pattern in &block.patterns {
        if glob(&pattern.text, name) {
            if pattern.negated {
                return false;
            }
            positive = true;
        }
    }
    positive
}

/// `*` 匹配任意串、`?` 匹配恰好一个字符，其余逐字符比 —— **区分大小写**（实测）。
fn glob(pattern: &str, name: &str) -> bool {
    fn go(pattern: &[char], name: &[char]) -> bool {
        match pattern.split_first() {
            None => name.is_empty(),
            Some(('*', rest)) => (0..=name.len()).any(|skip| go(rest, &name[skip..])),
            Some(('?', rest)) => !name.is_empty() && go(rest, &name[1..]),
            Some((ch, rest)) => name.first() == Some(ch) && go(rest, &name[1..]),
        }
    }
    let pattern: Vec<char> = pattern.chars().collect();
    let name: Vec<char> = name.chars().collect();
    go(&pattern, &name)
}

/// 没有通配符，也没有取反前缀。
fn is_literal(pattern: &str) -> bool {
    !pattern.contains(['*', '?'])
}

fn finding(line: usize, keyword: &str, message: &str) -> Finding {
    Finding {
        line,
        keyword: keyword.to_owned(),
        message: message.to_owned(),
    }
}

/// `IdentityFile` 那条警告的说法（私钥不导入 —— plan 0506 的非目标）。
///
/// 措辞在 plan 0903 之后改过一次：池里已经有一把**同名**的钥匙时（从 Bitwarden 导入进来的
/// 那些），导入会把条目接到它上面（[`crate::pools::import`] 的模块文档写了那条规则）。
/// 于是这句话不能再说"连接时走 ssh-agent" —— 那只在**没接上**的时候成立。两种情形在这句里
/// 都说得通，而究竟发生了哪一种由导入报告里那条 `linked` 说明。
const IDENTITY_FILE: &str = "这个密钥文件（私钥）本身没有导入：本产品的私钥住在密钥池里。\
                            池里若已有一把**同名**的钥匙（例如从 Bitwarden 导入的那一把），\
                            这个条目会自动接上它；没有同名钥匙时条目按 `publickey` 落库、\
                            连接时走 ssh-agent";

// ── 三档分类 ────────────────────────────────────────────────────────────────

/// 一条指令算哪一档。
enum Class {
    /// 六条之一。
    Supported,
    /// 认识它，但**不导入**：只影响本地行为，逐条警告后继续。
    Ignored(Category),
    /// 整份报错（`&'static str` 是给用户看的理由）。
    Refused(&'static str),
}

/// 第二档的分组。一条一条列关键字太长，而**理由是按组成立的** —— 所以理由挂在类别上。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Category {
    KeepAlive,
    Algorithms,
    Credentials,
    KnownHosts,
    Forward,
    Local,
    Session,
    Multiplexing,
}

impl Category {
    fn message(self) -> &'static str {
        match self {
            // 每一句都要回答同一个问题："忽略它，会不会连到**别的机器**、或以**别的身份**上去？"
            // 答"不会"才配待在这一档（答案见 ADR-0003 D14 的三档表）。
            Self::KeepAlive => "保活与超时只影响这条连接自己的寿命，不影响连到哪台机器",
            Self::Algorithms => {
                "算法与压缩不改目的地：我们用自己的默认取值（可能因此连不上，但不会连错）"
            }
            Self::Credentials => {
                "认证方式与凭据来源不改目的地、也不改登录身份：我们按自己的顺序试（密钥池 → ssh-agent → 交互输入）"
            }
            Self::KnownHosts => {
                "主机密钥记在哪、怎么记，我们有自己的规矩（库内缓存 + 只读的用户 known_hosts）：这几条只会让我们更严"
            }
            Self::Forward => "转发是另一套机制（阶段 6 的隧道池）：这一条不会替我们建转发",
            Self::Local => "本地日志与呈现选项：我们没有对应的开关",
            Self::Session => "会话形态与本地/远端命令：我们只开一个普通的交互式 shell",
            Self::Multiplexing => "连接复用：我们不共用连接（一条会话一条连接）",
        }
    }
}

/// 第二档：**整份删掉这些行也不会改变目的地与登录身份**的关键字（OpenSSH 10.5 的定义）。
///
/// ⚠️ 加一个新关键字进来之前先问：忽略它会不会**换一台机器**、**换一个身份**、或**放宽一道
/// 用户设的闸**？三问里有一个"会"，它就该去第三档。别把它当成"见过的名字都往里扔"。
const IGNORED_KEEPALIVE: &[&str] = &[
    "channeltimeout",
    "connectionattempts",
    "connecttimeout",
    "serveralivecountmax",
    "serveraliveinterval",
    "tcpkeepalive",
];
const IGNORED_ALGORITHMS: &[&str] = &[
    "casignaturealgorithms",
    "ciphers",
    "compression",
    "hostbasedacceptedalgorithms",
    "hostbasedkeytypes",
    "hostkeyalgorithms",
    "ipqos",
    "kexalgorithms",
    "macs",
    "pubkeyacceptedalgorithms",
    "pubkeyacceptedkeytypes",
    "rekeylimit",
    "requiredrsasize",
    "versionaddendum",
];
const IGNORED_CREDENTIALS: &[&str] = &[
    "addkeystoagent",
    "certificatefile",
    "challengeresponseauthentication",
    "gssapiauthentication",
    "gssapidelegatecredentials",
    "hostbasedauthentication",
    "identitiesonly",
    "identityagent",
    "kbdinteractiveauthentication",
    "numberofpasswordprompts",
    "passwordauthentication",
    "pkcs11provider",
    "preferredauthentications",
    "pubkeyauthentication",
    "securitykeyprovider",
    "usekeychain",
];
const IGNORED_KNOWN_HOSTS: &[&str] = &[
    "checkhostip",
    "globalknownhostsfile",
    "hashknownhosts",
    "knownhostscommand",
    "nohostauthenticationforlocalhost",
    "stricthostkeychecking",
    "updatehostkeys",
    "userknownhostsfile",
    "verifyhostkeydns",
];
const IGNORED_FORWARD: &[&str] = &[
    "clearallforwardings",
    "dynamicforward",
    "enablesshkeysign",
    "exitonforwardfailure",
    "forwardagent",
    "forwardx11",
    "forwardx11timeout",
    "forwardx11trusted",
    "gatewayports",
    "localforward",
    "permitremoteopen",
    "remoteforward",
    "streamlocalbindmask",
    "streamlocalbindunlink",
];
const IGNORED_LOCAL: &[&str] = &[
    "enableescapecommandline",
    "escapechar",
    "fingerprinthash",
    "loglevel",
    "logverbose",
    "obscurekeystroketiming",
    "syslogfacility",
    "visualhostkey",
    "xauthlocation",
];
const IGNORED_SESSION: &[&str] = &[
    "batchmode",
    "forkafterauthentication",
    "localcommand",
    "permitlocalcommand",
    "remotecommand",
    "requesttty",
    "sendenv",
    "sessiontype",
    "setenv",
    "stdinnull",
    "tag",
];
const IGNORED_MULTIPLEXING: &[&str] = &["controlmaster", "controlpath", "controlpersist"];

/// 第三档里"**会改目的地或解析路径**"的那些。
const REFUSED_DESTINATION: &[&str] = &[
    "addressfamily",
    "bindaddress",
    "bindinterface",
    "canonicalizedomains",
    "canonicalizefallbacklocal",
    "canonicalizehostname",
    "canonicalizemaxdots",
    "canonicalizepermittedcnames",
    "proxycommand",
    "proxyusefdpass",
    "tunnel",
];

fn classify(keyword: &str) -> Class {
    if keyword == "host" || supported(keyword) {
        return Class::Supported;
    }
    if let Some(category) = ignored(keyword) {
        return Class::Ignored(category);
    }
    if REFUSED_DESTINATION.contains(&keyword) {
        return Class::Refused(
            "这条会改变**连到哪台机器**（代理 / 名字改写 / 源地址）—— 按它连接与按我们连接可能不是同一台",
        );
    }
    match keyword {
        "include" => Class::Refused(
            "被包含的文件不在我们读到的这段文本里，而它可以出现在 `Host` 段**内部** —— \
             按我们看到的连，与按它连可能是两回事",
        ),
        "hostkeyalias" | "revokedhostkeys" => {
            Class::Refused("这条改变我们**信任哪把主机密钥**（查表用的名字 / 吊销名单），不能忽略")
        }
        "refuseconnection" => {
            Class::Refused("配置里写着**拒绝连接**这一台，而忽略它就等于照样连过去")
        }
        "ignoreunknown" => {
            Class::Refused("'放行某条指令'的语义：我们不接受放过任何一条不生效的指令，请把它删掉")
        }
        _ => Class::Refused("这是本程序不认识的指令（受限子集只认六条）"),
    }
}

fn supported(keyword: &str) -> bool {
    matches!(
        keyword,
        "hostname" | "user" | "port" | "identityfile" | "proxyjump"
    )
}

fn ignored(keyword: &str) -> Option<Category> {
    let table: [(&[&str], Category); 8] = [
        (IGNORED_KEEPALIVE, Category::KeepAlive),
        (IGNORED_ALGORITHMS, Category::Algorithms),
        (IGNORED_CREDENTIALS, Category::Credentials),
        (IGNORED_KNOWN_HOSTS, Category::KnownHosts),
        (IGNORED_FORWARD, Category::Forward),
        (IGNORED_LOCAL, Category::Local),
        (IGNORED_SESSION, Category::Session),
        (IGNORED_MULTIPLEXING, Category::Multiplexing),
    ];
    table
        .into_iter()
        .find(|(keywords, _)| keywords.contains(&keyword))
        .map(|(_, category)| category)
}
