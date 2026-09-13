//! `~/.ssh/config` 受限子集解析的**判据本身**（plan 0506 / ADR-0003 D14）。
//!
//! 这一个文件盯三件事，每一件的期望值都有实测依据（系统 OpenSSH 10.5 的 `ssh -G`，
//! 见 plan 的「判据」一节）：
//!
//! | 盯什么 | 为什么它重要 |
//! |---|---|
//! | **首次取到的值生效**（含 `Host *` 与全局段） | 读成"一个 `Host` 块 = 一行"会**静默连错端口** |
//! | 三档分类（导入 / 警告 / 整份报错） | "受限"变成"悄悄错"就是这一条防线塌了 |
//! | `ProxyJump` 的三种写法与补建 | 导入进来的跳板要能**真连**（plan 0505 才刚做到） |
//!
//! 解析器是纯函数，所以这里不需要库、app、网络，也不需要 `VICTAURI_E2E`。

#![allow(clippy::unwrap_used)] // 测试里的 unwrap 是断言手段（root Cargo.toml 的 lints 约定）

use akasha_store::sshconfig::{Imported, Target, parse};

/// 本机用户名在解析器里是**调用方给的**（解析器不读环境），测试里就写死一个。
const ME: &str = "tester";

fn ok(text: &str) -> Imported {
    match parse(text, ME) {
        Ok(imported) => imported,
        Err(problems) => panic!(
            "这份配置本该能导入，却报了 {} 条问题：{problems:#?}",
            problems.len()
        ),
    }
}

fn refused(text: &str) -> Vec<akasha_store::sshconfig::Finding> {
    match parse(text, ME) {
        Ok(imported) => panic!("这份配置本该整份报错，却导入了 {:?}", imported.targets),
        Err(problems) => problems,
    }
}

fn target<'a>(imported: &'a Imported, name: &str) -> &'a Target {
    imported
        .targets
        .iter()
        .find(|target| target.name == name)
        .unwrap_or_else(|| panic!("没有 {name} 这个条目：{:#?}", imported.targets))
}

// ── 一条正常的配置 ──────────────────────────────────────────────────────────

#[test]
fn a_plain_entry_reads_the_six_directives() {
    let imported = ok("\
        Host work\n\
        \x20   HostName work.example.com\n\
        \x20   User alice\n\
        \x20   Port 2222\n\
        \x20   IdentityFile ~/.ssh/id_work\n\
        \x20   ProxyJump bastion\n\
        Host bastion\n\
        \x20   HostName 10.0.0.1\n\
        \x20   User root\n");

    let work = target(&imported, "work");
    assert_eq!(work.host, "work.example.com");
    assert_eq!(work.user, "alice");
    assert_eq!(work.port, 2222);
    assert_eq!(work.key_file.as_deref(), Some("~/.ssh/id_work"));
    assert_eq!(work.jump.as_deref(), Some("bastion"));
    assert!(!work.provisional);

    let bastion = target(&imported, "bastion");
    assert_eq!(bastion.host, "10.0.0.1");
    assert_eq!(bastion.user, "root");
    assert_eq!(bastion.port, 22, "没写 Port 就是 22");
    assert_eq!(bastion.jump, None);

    // 密钥文件没有导入这件事**必须在报告里露面**（不是静默丢掉一个字段）。
    assert_eq!(imported.ignored.len(), 1, "{:#?}", imported.ignored);
    assert_eq!(imported.ignored[0].keyword, "identityfile");
    assert_eq!(imported.ignored[0].line, 5);
}

#[test]
fn a_name_without_host_name_is_the_name_itself() {
    let imported = ok("Host MiXeD\n  User alice\n");
    // 大小写按 `ssh -G` 实测：没写 `HostName` 时 OpenSSH 把目标名小写化了
    // （`ssh -G MiXeD` → `hostname mixed`），而 `Host` 模式匹配用的是原样。
    assert_eq!(target(&imported, "MiXeD").host, "mixed");
    assert_eq!(target(&imported, "MiXeD").user, "alice");
}

#[test]
fn a_name_without_user_gets_the_local_one() {
    let imported = ok("Host work\n  HostName work.example.com\n");
    assert_eq!(target(&imported, "work").user, ME);
}

// ── 首次取到的值生效（`ssh -G` 实测的那几条） ────────────────────────────────

#[test]
fn the_first_value_wins_across_the_whole_file() {
    // 实测：`Host *` + `Port 2222` 写在前面时，后面 `Host foo` 的 `Port 33` **不生效**。
    // 这是"一个 Host 块 = 一行"那种读法会静默连错端口的地方。
    let imported = ok("\
        Host *\n\
        \x20   Port 2222\n\
        Host foo\n\
        \x20   Port 33\n\
        \x20   HostName foo.example\n");

    assert_eq!(target(&imported, "foo").port, 2222);
    assert_eq!(target(&imported, "foo").host, "foo.example");
}

#[test]
fn directives_before_the_first_host_block_are_global() {
    // 实测：文件开头的 `User` / `Port` 对每一台都成立（等价于一个写在前面的 `Host *`）。
    let imported = ok("\
        User preuser\n\
        Port 2222\n\
        Host foo\n\
        \x20   HostName foo.example\n\
        Host bar\n\
        \x20   HostName bar.example\n");

    assert_eq!(target(&imported, "foo").user, "preuser");
    assert_eq!(target(&imported, "foo").port, 2222);
    assert_eq!(target(&imported, "bar").user, "preuser");
    assert_eq!(target(&imported, "bar").port, 2222);
}

#[test]
fn a_wildcard_block_is_not_an_entry_but_its_values_apply() {
    let imported = ok("\
        Host *.corp\n\
        \x20   User wild\n\
        \x20   ProxyJump bastion\n\
        Host a.corp\n\
        \x20   HostName a.internal\n\
        Host bastion\n\
        \x20   HostName 10.0.0.1\n");

    let names: Vec<&str> = imported.targets.iter().map(|t| t.name.as_str()).collect();
    assert_eq!(names, ["a.corp", "bastion"], "通配块自己不是一个条目");
    assert_eq!(
        target(&imported, "a.corp").user,
        "wild",
        "通配块的取值并入了匹配到的条目"
    );
    assert_eq!(target(&imported, "a.corp").jump.as_deref(), Some("bastion"));
    assert!(
        imported.notes.iter().any(|note| note.contains("通配块")),
        "通配块要被写明：{:#?}",
        imported.notes
    );
}

#[test]
fn a_negated_pattern_excludes_a_host_from_the_block() {
    let imported = ok("\
        Host *.corp !bad.corp\n\
        \x20   Port 2222\n\
        Host bad.corp\n\
        \x20   HostName bad.internal\n\
        Host good.corp\n\
        \x20   HostName good.internal\n");

    assert_eq!(target(&imported, "good.corp").port, 2222);
    assert_eq!(
        target(&imported, "bad.corp").port,
        22,
        "取反命中的那一台不吃这一段"
    );
}

#[test]
fn patterns_are_case_sensitive_while_keywords_are_not() {
    // 实测：`Host foo` **不**匹配命令行上的 `FOO`（`ssh -G FOO` 仍是 `hostname FOO` 的小写）。
    let imported = ok("\
        HOST foo\n\
        \x20   HOSTNAME lower.example\n\
        \x20   USER alice\n\
        Host FOO\n\
        \x20   HostName upper.example\n");

    assert_eq!(names(&imported), ["FOO", "foo"], "两个不同的字面条目");
    assert_eq!(target(&imported, "foo").host, "lower.example");
    assert_eq!(target(&imported, "FOO").host, "upper.example");
}

#[test]
fn repeated_host_lines_merge_into_one_entry() {
    let imported = ok("\
        Host foo\n\
        \x20   HostName foo.example\n\
        Host foo\n\
        \x20   User alice\n\
        \x20   HostName other.example\n");

    assert_eq!(names(&imported), ["foo"], "重复的 `Host` 是同一个条目");
    assert_eq!(
        target(&imported, "foo").host,
        "foo.example",
        "先写的那一行胜"
    );
    assert_eq!(target(&imported, "foo").user, "alice");
}

#[test]
fn one_host_line_with_two_patterns_makes_two_entries() {
    let imported = ok("Host foo bar\n  User alice\n  HostName shared.example\n");
    assert_eq!(names(&imported), ["bar", "foo"]);
    assert_eq!(target(&imported, "foo").host, "shared.example");
    assert_eq!(target(&imported, "bar").host, "shared.example");
}

// ── 文本约定：注释、引号、`=` ────────────────────────────────────────────────

#[test]
fn comments_quotes_and_the_equals_separator_are_understood() {
    let imported = ok("\
        # 整行注释\n\
        Host=\"my host\"   # 行尾注释\n\
        \x20   HostName=host.example   # 另一个\n\
        \x20   User = alice\n\
        \x20   Port=2222\n");

    assert_eq!(names(&imported), ["my host"]);
    assert_eq!(target(&imported, "my host").host, "host.example");
    assert_eq!(target(&imported, "my host").user, "alice");
    assert_eq!(target(&imported, "my host").port, 2222);
}

#[test]
fn a_hash_inside_quotes_is_not_a_comment() {
    let imported = ok("Host \"we#ird\"\n  User alice\n");
    assert_eq!(names(&imported), ["we#ird"]);
}

// ── 三档：整份报错 ──────────────────────────────────────────────────────────

#[test]
fn match_is_a_whole_file_error() {
    let problems = refused(
        "\
        Host m\n\
        \x20   HostName m.example\n\
        Match host m\n\
        \x20   User matched\n",
    );

    assert_eq!(problems.len(), 1);
    assert_eq!(problems[0].line, 3);
    assert_eq!(problems[0].keyword, "match");
}

#[test]
fn include_is_a_whole_file_error_even_inside_a_host_block() {
    let problems = refused(
        "\
        Host i\n\
        \x20   HostName i.example\n\
        \x20   Include extra.conf\n",
    );

    assert_eq!(problems.len(), 1);
    assert_eq!(problems[0].line, 3);
    assert_eq!(problems[0].keyword, "include");
}

#[test]
fn a_directive_that_changes_the_destination_is_refused() {
    for line in [
        "ProxyCommand nc %h %p",
        "CanonicalizeHostname yes",
        "BindAddress 10.0.0.5",
        "AddressFamily inet6",
        "HostKeyAlias other",
        "RevokedHostKeys /tmp/revoked",
        "IgnoreUnknown UseKeychain",
        "RefuseConnection 别连这台",
    ] {
        let config = format!("Host foo\n  HostName foo.example\n  {line}\n");
        let problems = refused(&config);
        assert_eq!(problems.len(), 1, "`{line}` 该被拒：{problems:#?}");
        assert_eq!(problems[0].line, 3, "`{line}` 的行号");
    }
}

#[test]
fn an_unknown_keyword_is_refused_by_default() {
    // 这一条是整张表的方向盘：**真有一条我们漏判的关键字，它落在这里**（保守的那一边）。
    let problems = refused("Host foo\n  UseSomeFutureThing yes\n");
    assert_eq!(problems[0].keyword, "usesomefuturething");
    assert!(
        problems[0].message.contains("不认识"),
        "{}",
        problems[0].message
    );
}

#[test]
fn every_problem_is_reported_at_once() {
    // 一次列全是给用户的：一份配置改三处，不该跑三趟。
    let problems = refused(
        "\
        Host foo\n\
        \x20   Match host foo\n\
        \x20   ProxyCommand nc %h %p\n\
        \x20   WeirdKeyword 1\n",
    );

    assert_eq!(problems.len(), 3, "{problems:#?}");
    assert_eq!(
        problems
            .iter()
            .map(|problem| problem.line)
            .collect::<Vec<_>>(),
        [2, 3, 4]
    );
}

#[test]
fn a_malformed_value_is_refused() {
    for line in ["Port 70000", "Port abc", "Port", "User a b"] {
        let config = format!("Host foo\n  {line}\n");
        let problems = refused(&config);
        assert_eq!(problems[0].line, 2, "`{line}` 该被拒");
    }
}

// ── 三档：警告并继续 ────────────────────────────────────────────────────────

#[test]
fn a_local_only_directive_warns_and_the_rest_still_imports() {
    let imported = ok("\
        Host *\n\
        \x20   AddKeysToAgent yes\n\
        \x20   ServerAliveInterval 30\n\
        \x20   StrictHostKeyChecking no\n\
        \x20   LocalForward 8080 localhost:80\n\
        \x20   LogLevel DEBUG\n\
        Host foo\n\
        \x20   HostName foo.example\n");

    assert_eq!(names(&imported), ["foo"], "警告不挡导入");
    let keywords: Vec<&str> = imported
        .ignored
        .iter()
        .map(|finding| finding.keyword.as_str())
        .collect();
    assert_eq!(
        keywords,
        [
            "addkeystoagent",
            "serveraliveinterval",
            "stricthostkeychecking",
            "localforward",
            "loglevel"
        ]
    );
    // 每条都要有一句"为什么它能被忽略"，且**行号**要对得上（用户要能照着改）。
    assert_eq!(imported.ignored[0].line, 2);
    assert!(
        imported.ignored[0].message.contains("认证方式"),
        "{:#?}",
        imported.ignored[0]
    );
    assert!(
        imported.ignored[2].message.contains("更严"),
        "{:#?}",
        imported.ignored[2]
    );
}

#[test]
fn a_keyword_that_differs_only_in_case_is_the_same_directive() {
    let imported = ok("Host foo\n  HOSTNAME foo.example\n  PORT 2222\n");
    assert_eq!(target(&imported, "foo").host, "foo.example");
    assert_eq!(target(&imported, "foo").port, 2222);
}

// ── `ProxyJump` ─────────────────────────────────────────────────────────────

#[test]
fn proxyjump_none_means_a_direct_connection() {
    let imported = ok("Host foo\n  ProxyJump none\n");
    assert_eq!(target(&imported, "foo").jump, None);
}

#[test]
fn proxyjump_with_a_comma_chain_hangs_each_hop_on_the_previous_one() {
    // `ProxyJump a,b` = 先连 a 再连 b：靠目标最近的是 b，b 的跳板是 a。
    let imported = ok("\
        Host target\n\
        \x20   HostName target.internal\n\
        \x20   ProxyJump a,b\n\
        Host a\n\
        \x20   HostName 10.0.0.1\n\
        Host b\n\
        \x20   HostName 10.0.0.2\n");

    assert_eq!(target(&imported, "target").jump.as_deref(), Some("b"));
    assert_eq!(target(&imported, "b").jump.as_deref(), Some("a"));
    assert_eq!(target(&imported, "a").jump, None);
}

#[test]
fn a_jump_host_with_no_host_block_gets_one_built_for_it() {
    // 常见写法：跳板只在 `ProxyJump` 里出现一次，自己没有 `Host` 块。
    let imported = ok("\
        Host target\n\
        \x20   HostName target.internal\n\
        \x20   ProxyJump jump.example.com\n");

    let jump = target(&imported, "jump.example.com");
    assert_eq!(jump.host, "jump.example.com");
    assert_eq!(jump.port, 22);
    assert_eq!(jump.user, ME);
    assert!(
        jump.provisional,
        "补建的要打上标记：池里已有同名行时它不覆盖"
    );
    assert_eq!(
        target(&imported, "target").jump.as_deref(),
        Some("jump.example.com")
    );
    assert!(
        imported.notes.iter().any(|note| note.contains("补建")),
        "补建要写在报告里：{:#?}",
        imported.notes
    );
}

#[test]
fn a_built_jump_host_still_takes_the_global_values() {
    let imported = ok("\
        User ops\n\
        Port 2200\n\
        Host target\n\
        \x20   ProxyJump jump.example.com\n");

    let jump = target(&imported, "jump.example.com");
    assert_eq!(jump.user, "ops", "补建的条目走同一个求值器");
    assert_eq!(jump.port, 2200);
}

#[test]
fn proxyjump_with_a_user_or_port_is_refused_with_a_way_out() {
    for spec in ["alice@bastion", "bastion:2222", "ssh://bastion"] {
        let config = format!("Host foo\n  ProxyJump {spec}\n");
        let problems = refused(&config);
        assert_eq!(problems[0].keyword, "proxyjump");
        assert!(
            problems[0].message.contains("Host` 块"),
            "报错要给出改法：{}",
            problems[0].message
        );
    }
}

#[test]
fn a_jump_chain_that_loops_back_is_refused() {
    let problems = refused(
        "\
        Host *\n\
        \x20   ProxyJump bastion\n\
        Host bastion\n\
        \x20   HostName 10.0.0.1\n",
    );

    assert!(
        problems
            .iter()
            .any(|problem| problem.message.contains("成环")),
        "{problems:#?}"
    );
}

#[test]
fn two_writings_that_disagree_about_the_same_hop_are_refused() {
    let problems = refused(
        "\
        Host b\n\
        \x20   ProxyJump a\n\
        Host a\n\
        \x20   HostName 10.0.0.1\n\
        Host c\n\
        \x20   ProxyJump a\n\
        Host target\n\
        \x20   ProxyJump b,c\n",
    );

    assert!(
        problems
            .iter()
            .any(|problem| problem.message.contains("不同的跳板")),
        "{problems:#?}"
    );
}

// ── 空配置文件 ──────────────────────────────────────────────────────────────

#[test]
fn a_file_with_no_concrete_host_is_not_an_error_but_says_so() {
    let imported = ok("# 只有注释与全局默认值\nHost *\n  User alice\n");
    assert!(imported.targets.is_empty());
    assert!(
        imported
            .notes
            .iter()
            .any(|note| note.contains("没有具体的主机条目")),
        "{:#?}",
        imported.notes
    );
}

#[test]
fn an_empty_file_is_an_empty_import() {
    let imported = ok("");
    assert!(imported.targets.is_empty());
    assert!(imported.ignored.is_empty());
}

fn names(imported: &Imported) -> Vec<&str> {
    imported.targets.iter().map(|t| t.name.as_str()).collect()
}
