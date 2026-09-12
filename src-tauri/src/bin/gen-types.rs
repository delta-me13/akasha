//! 代码生成入口：`just gen-types` 跑的就是它。
//!
//! 为什么是一个 bin 而不是测试或 `build.rs`：
//!
//! * `build.rs` 每次编译都写文件，会让"生成物没提交"这件事变得无法察觉（它总是最新的）；
//! * 测试里写仓库文件是副作用，跑一次 `cargo test` 就改动工作区；
//! * bin 是显式的：谁跑、什么时候跑、写到哪，都在命令里看得见。

use std::fs;
use std::io::Write;
use std::path::Path;

// 这里用 `writeln!` 而不是 `println!`：`no-println` 规则（`AGENTS.md` §3.4）守的是
// **运行期日志**必须进 tracing；本文件是一次性代码生成工具，没有 app 在跑，
// 也没有日志管道可进，输出的是跑完给人看的结果。
fn report(line: &str) {
    let mut out = std::io::stdout();
    let _ = writeln!(out, "{line}");
}

fn main() {
    let path = Path::new(akasha_lib::bindings::BINDINGS_PATH);
    if let Some(parent) = path.parent()
        && let Err(err) = fs::create_dir_all(parent)
    {
        report(&format!("❌ 无法创建 {}：{err}", parent.display()));
        std::process::exit(1);
    }

    let exported =
        akasha_lib::bindings::builder().export(specta_typescript::Typescript::default(), path);

    match exported {
        Ok(()) => report(&format!("✅ 已生成 {}", path.display())),
        Err(err) => {
            // 生成失败不能只打印 —— 那会让 `just gen-types && git diff` 这类用法
            // 在"其实什么都没生成"的情况下继续往下走。
            report(&format!("❌ 生成失败：{err}"));
            std::process::exit(1);
        }
    }
}
