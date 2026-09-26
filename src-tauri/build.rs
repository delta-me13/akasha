fn main() {
    // Windows（MSVC）上多一条链接器开关：`/IGNORE:4099`。
    //
    // 为什么要它：vendored OpenSSL（`openssl-src`）在 MSVC 下用 `/Zi /Fdossl_static.pdb` 编译，
    // 而那份 PDB 不随 `libopenssl_sys-*.rlib` 一起发到 `deps/` 下 —— 于是**每一个** .obj 都让
    // 链接器打一行 `warning LNK4099: PDB 'ossl_static.pdb' was not found ...`（本机实测：链接
    // 一个测试目标 1618 行；CI 上三十多个目标加上万行，把真正的报错淹掉）。缺的那份调试信息
    // 本来就是链接器自己也说"当作没有调试信息"继续的那一类，所以关掉这一类警告。
    //
    // 判据用 `CARGO_CFG_TARGET_*`（而不是 `cfg!(windows)`）：交叉编译时 host 与 target 可以不同。
    // 走 `rustc-link-arg` 而不是 `.cargo/config.toml` 的 `rustflags`：后者会让**整棵依赖树**
    // 重新编译（含从源码构建的 OpenSSL），而这里只影响本包自己的 bin / test / example 的链接。
    if std::env::var("CARGO_CFG_TARGET_OS").as_deref() == Ok("windows")
        && std::env::var("CARGO_CFG_TARGET_ENV").as_deref() == Ok("msvc")
    {
        // cargo 从构建脚本的 stdout 读指令，`println!` 在这里就是它的 API —— 见 `no-println`
        // 规则的 ignore 列表（那条规则管的是运行期日志，构建脚本不在其中）。
        println!("cargo:rustc-link-arg=/IGNORE:4099");
    }

    tauri_build::build()
}
