# 已知问题与教训

> **本文件是这些条目的唯一住处**：从 `docs/STATUS.md` 逐字移来（2026-09-27），编号**永不复用**。
> 新增条目写在这里，并在 [`STATUS.md`](./STATUS.md) 的「已知问题（开放的）」索引里补一行（仅当它仍然开放）。
>
> 三类去处不要混：**仍开放、写判据或写代码时会用到的** → STATUS 的索引；**已处置的**
> （标了 `已修` / `已放弃` / `已关闭`）→ [`archive/status-history.md`](./archive/status-history.md)；
> 其余（工具链怪癖、上游行为、方法论教训）留在本文件 —— 它们多数已经被 `AGENTS.md` 的某条规则
> 或某个 ADR 吸收，留在这里是为了回答「这条规则当时是怎么来的」。

## 条目

1. **`victauri-test` 生成的测试需要消费方自己加 `tokio` dev-dependency**。

2. **`cargo deny init` 模板里 `[licenses] allow = []` 的含义是"拒绝一切许可证"**。

3. **just 的变量写 `$p`，不是 Make 的 `$$p`**。

4. **just 用 justfile 所在目录作为配方工作目录**。

5. **系统库缺失只在 cargo 构建脚本阶段暴露**；本机是 CachyOS（Arch 系），不是 apt。

6. **Tauri 没有 Rust 热重载，Victauri 也不提供**。

7. **just 的 shebang 配方需要可写的 runtime dir**，受限环境会失败。

8. **仓库根没有 `Cargo.toml`** → 根目录下一切 cargo 命令失败（**临时脚本也算**）。

9. **`victauri-test` 生成的 `tests/*.rs` 不符合 rustfmt 默认风格** —— 运行一次 `just fmt`。

10. **CI 里 `libappindicator3-dev` 已不存在**，要用 `libayatana-appindicator3-dev`。

11. **受限环境下"写工作区之外被拒"看起来像工具/代码故障** —— 识别 → **直接提权重试**。

12. **`git checkout <file>` 会静默丢弃未提交的改动** —— 负例自检用 `cp` 备份/还原
    （⚠️ 备份**不得**放 `/tmp`：沙箱每次调用一个私有 `/tmp`）。

13. **正向校验若不限定到目标段落就形同虚设** —— `docs-check` 用 `awk` 取 §2。

14. **`docs-check` 的反向检查**已扩到 `AGENTS.md` / `README.md` / `ROADMAP.md` / `docs/**/*.md`
    （`CLAUDE.md` 除外，理由见问题 #153）。
    ⚠️ **它不认识"下一步才存在"的命令**：plan 里出现的 `just <新配方>` 会让 docs-check 失败 ——
    展开 plan 的那次提交要么先把配方落上，要么**先不写那个命令名**（plan 落地时就地补）。

15. **后台遗留的 `just dev` 会让 Vite 继续监听 1420 而 app 早已不在**。

16. **含反引号的 grep 模式在 justfile 配方里必须整体放进单引号**。

17. **多文件行数检查要逐文件取**（awk 的 `NR` 会跨文件累加）。

18. **`[profile.*]` 写在 workspace 成员里会被静默忽略**。

19. **整体 `mv` 构建缓存会留下写死的绝对路径**。

20. **`cargo` 在成员目录里只选当前包** —— crate 级配方不写 `--workspace` 会**静默漏掉**成员。

21. **tauri CLI 默认只监听 `src-tauri`** —— 成员放外面 = 开发循环静默失效。

22. **cargo 的空 glob 是硬错误**（`members = ["crates/*"]`）。

23. **justfile 里不能出现完整的 `{{ … }}`**（要写字面量用 `{{{{`）。

24. **"一份工作流同时服务两个 forge"会持续产生额外维护成本**。

25. **兼容层遗留的代码会以"看起来更稳"的形式保留下来**。

26. **阻塞的 `Read` 与"按时间交付"在机制上冲突** —— 正解是读线程 + `recv_timeout(期限)`。

27. **零匹配的测试过滤器在 nextest 里是"报错"**。

28. **`cargo bench` 会同时以 bench 模式运行单测目标**（正常行为）。

29. **仓库里出现第二个 bin 会让 `tauri dev` 无法启动** —— 修法是 `default-run`。

30. **只写 `path` 的依赖等于版本号写 `*`** —— path 依赖要**同时写 `version`**。

31. **`Channel<Vec<u8>>` 不是二进制通道** —— 真正走 raw 的只有
    `Channel<InvokeResponseBody>` + `InvokeResponseBody::Raw`。**不得只按字节数验收**。

32. **`u64` 不能直接过 IPC**：改用壳层 `u32` 句柄 + **checked** 转换。⚠️ 被生成器拒绝的是
    **一整类**（`usize` / `isize` / `i64` / `u64` / `i128` / `u128`）。

33. **沙箱里 E2E 必须与 app 在**同一次** bash 调用内**（每次调用都是独立的 bwrap）。

34. **`pkill -f <模式>` 会匹配到自身** —— 用 `pkill -f '[v]ite'` 或按 PID/进程组终止。

35. **`@xterm/addon-unicode11` 需要 `allowProposedApi: true`**。

36. **不在门禁里的测试等于没测**：`just test-e2e` 既不在 `ready` 里、又要真实 app。

37. **`git mv` 之后 `docs-check` 会同时验两件事**（文件在不在、索引指得对不对）。

38. **每加一个依赖就多一份要维护的放行**。

39. **"测试自己抛的异常"会污染同一 app 上后运行的用例**。

40. **vite 默认只监听 `[::1]:1420`**；`/tmp/victauri/<pid>/` 里的 `pid` **就是 app 的 pid**。

41. **`(cmd) &` 在 fish 里是命令替换，不是子 shell**。

42. **`cargo test` 一次收多个 `--test` 时按目标名字母序运行**，不按参数顺序。

43. **`tauri-plugin-log` 默认的 `TargetKind::LogDir` 会让"日志目录不可写"变成 app 打不开**。

44. **`tracing/log-always` 会把依赖树的 TRACE 一起转成 `log` 记录**。

45. **SIGKILL 的投递是异步的**。

46. **portable-pty(unix) 的 `Child::kill()` 不是纯 SIGKILL**。

47. **早于日志插件注册的 `tracing` 事件会静默消失**。

48. **`/proc/<pid>` 存在 ≠ 进程还活着**（僵尸也有目录项）。

49. **按"命令行里含某段文本"找进程会误伤**。

50. **`term.dispose()`（xterm）会抛，而它运行在 React 的 effect 清理函数里**。

51. **多标签之后 DOM 选择器不再唯一**。

52. **断言超时不一定是"慢"**：真实原因可能是**界面已经被卸载**。

53. **`tauri-specta` 的事件必须 `mount_events`**。

54. **官方 `Channel` 不提供"流已结束"的通知**。

55. **会话中还有其它进程持有 PTY 时，主端读不到 EOF**。

56. **接线一次的回调必须走 `ref`**。

57. **日志消息中的"括号解释"会持续膨胀**。

58. **`tauri-plugin-log` 默认 formatter 的时间戳只到秒**。

59. **`signal` 字段的值是本地化的**（zh_CN 下 `SIGKILL` 写成 `已杀死`），尚未修复。

60. **托盘在 Linux 上需要写盘** —— 只读 runtime dir 中无法创建，因此它只能是**可选能力**。

61. **`libayatana-appindicator3` 与老的 `libappindicator3` 都是运行时 dlopen**。

62. **dbusmenu 的 item id 会随菜单重建而改变**。

63. **SNI 注册用的是唯一名**（`:1.x`）。

64. **Victauri 的 `window` 工具能机器验证窗口状态**。

65. **`AppHandle::exit()` 也会触发 `RunEvent::ExitRequested`**。

66. **便携数据目录就在 bin 同目录**，而 `just dev` 与 `just test-e2e` **共用同一个 bin**。

67. **Victauri 的 REST 兜底接口返回的是 `{"result": …}` 包了一层**（而 `just test-e2e` 里的
    `VictauriClient` 走 MCP，`call_tool` 直接返回工具内容本身）。

68. **`/proc/<pid>/exe` 可能带 ` (deleted)` 后缀**。

69. **文档中的"事实"若不核对会持续膨胀**。

70. **cargo 的 workspace lint 继承是"全有或全无"**，而 `forbid` 不能被 `allow` 覆盖。

71. **带 `links = "..."` 的原生库在依赖树里只能有一个版本**。

72. **SQLCipher 的空 key 不是"静默关闭加密"，而是"返回错误且不装载 codec"**。

73. **SQLite 自己建出来的库文件是 644**（umask 022），不是 0600。

74. **`PRAGMA cipher_settings` 的输出是一列 `pragma` 行**。

75. **cargo-deny 的图根是"manifest 指向的那个包"，不是整个 workspace**。

76. **"0 字节的库"不是"空库"，是"还没有密钥"**。

77. **`cipher_memory_security` 是进程级、单向的**。

78. **模块内的 `#[cfg(test)] mod tests` 也要自己 `allow(clippy::unwrap_used)`**。

79. **`/proc/<pid>/mem` 的读用 `FOLL_FORCE`，绕过页保护**。

80. **`/proc/self/smaps` 的字段不是处处都有，而且属性行带缩进**。

81. **子串匹配会使规则失效**：按 `_` 分词、整词比较，并配一对命中 / 诱饵负例。

82. **Victauri 的 `get_registry` 在本仓库是空的**（命令都没标 `#[inspectable]`）。

83. **clippy 会把"两个常量比较"的断言判为失败**（`assertions_on_constants`）—— 搬进 `const { … }`。

84. **E2E 配方两段可能运行在**不同**的数据目录里** —— 两段都先 `mkdir -p` 便携目录。

85. **集成测试的共用脚手架放 `tests/common/mod.rs`，但必须自己 `#![allow(dead_code)]`**。

86. **`pkill -f <pattern>` 会匹配到该命令自身的命令行**。

87. **"什么都没发生"这类判据最容易写成永真式** —— 先用负例确认该判据会失败。

88. **`sqlcipher_export` 写出来的文件默认是版本 0**（它不传递 `user_version`）。

89. **"导出另开一条实现路径"的代价最高**。

90. **扫描器本身位于它要扫描的地址空间内** —— 缓冲复用 + 读完即擦 + 真随机的针。

91. **给"要扫描的段"设上限 = 使应当看见的副本落在扫描窗口之外**（漏扫与未泄漏在判据上无法区分）。

92. **解锁期间 `VmLck` 涨的大头不是我们那一页**：`cipher_memory_security` 会给 SQLCipher 的
    每次分配 `mlock`（实测 152 kB 里 148 kB）。

93. **"发现目录出现" ≠ "app 就绪"**：Victauri 的插件 setup 比 app 自己的 `.setup()` 早，
    所以刚连上时 `lifecycle` probe 还是 `{"initialized":false}`。判据要"等那个字段自己出现"。
    同一类还有第二层：`invoke_command` 走 webview bridge，是**最后**才好的一个。

94. **`no-println` 规则豁免的是 `**/tests/**`，不是 `#[cfg(test)] mod tests`**。
    正解：单测的"跳过"换成**不依赖环境**的 fixture，要输出就放进 `tests/`。

95. **用 `chmod` 造"不可写"在单测中不成立** —— 改用**结构性**造法（路径指向普通文件之下，
    `ENOTDIR` 对 root 同样无法绕过）。

96. **clippy 的 `undocumented_unsafe_blocks` 本来就看私有项**（1.98 实测）。

97. **只加 `undocumented_unsafe_blocks` 会漏掉另一半**：要靠反向的
    `unnecessary_safety_comment` / `unnecessary_safety_doc` 才闭环。

98. **文档里的"或 X"最容易无依据地出现** —— **核查时把每个"或"当成一条待证断言**。

99. **"唯一一处"这类计数若不核对就会失准**：要么写成"唯一**允许**的 crate"（结构性表述），
    要么就不在注释里写数字。

100. **照搬外部规范时要分清"结构"与"语种"**：解释写中文，只将**标签字面量**固定。

101. **断言型正则要按"节点实际文本"写**（`^Channel$` 匹配不到 `tauri::ipc::Channel<Vec<u8>>`）。

102. **探针必须放进规则 `files:` 覆盖的真实路径，且正例与诱饵都要有**。

103. **"词汇表"规则里，词边界是规则的一部分**（裸 `(Tab|Pane|Window|View)` 会误伤 `Table`）。

104. **`ast-grep test` 只测规则逻辑，不测 `files:` / `ignores:`** —— 路径范围仍要真实路径探针。

105. **`cargo add` / `cargo search` 需要写 `~/.cargo` 的索引缓存**：沙箱下该目录只读 → 按问题 #11 提权重试。

106. **`cargo deny` 会为 `cargo metadata` 拉取其它平台的依赖**（`russh` 会带出 `pageant`）。

107. **edition 2024 的 `impl Trait` 会捕获输入生命期** —— 不使用 `run_on_socket`，自行编写 accept 循环。

108. **`Transport::output_stream()` 只能获取一次**（两个读端会互相窃取字节）。

109. **`std::env::set_var` 在 Rust 2024 中是 `unsafe`**：依赖环境变量开关的行为无法在测试中构造前提；
    正解是把它变成**输入**（`SshAuth::agent_socket`）。

110. **`cargo nextest run -p <crate> <关键词>` 过滤的是测试的**函数名**，不是文件名** ——
    按文件过滤需使用 `--test <目标名>`。过滤器写错会使判据**永远不执行**。

111. **上游 `russh` 的 `Error::KeyChanged { line }` 在跳过注释行时不会递增行号** —— 要给出真实行号
    需自行计数（`true_line_of`）。

112. **`thiserror` 会把名为 `source` 的字段当作错误源**（插值会编译失败）—— 改用其它字段名。

113. **`// SAFETY:` 的位置即其含义**：写成 `/// SAFETY:` 挂在安全函数上会同时触发两条 lint。

114. **迁移必须在改动前先按*旧*版本校验形状**（顺序：读版本 → 按该版本查表 → 迁移 → 再查表）。

115. **`i64` 与 `u64` 一样无法通过 IPC**：以 `u32` 代理 + **checked** 转换。

116. **不得将 IPC 投影类型命名为 `*View`**：本仓库的 `no-ui-vocab-in-types` 会命中该词边界。

117. **tokio 1.53 的 `Runtime::handle()` 返回 `&Handle`** —— 存入结构体需 `.clone()`。

118. **React StrictMode（仅开发模式）会执行 effect 两遍**：类似"建立一个会话"的副作用因此发生两次；
    SSH 的两次提示会同时显示在同一面板中，表现为"点击 SSH 后一直连不上"。处置：连接**无条件推迟
    一个微任务**再发起。

119. **沙箱中"同一次 bash 调用"的边界包含重定向写出的日志文件** —— 写入 `/tmp` 后下次调用读不到，
    需写入**工作区**。

120. **`russh` 的 `Config::nodelay` 只在 `client::connect` 中生效**：我们两条路都使用
    `connect_stream`，因此"把 `config.nodelay` 设为 `true`"在这条路上**未生效**
    （plan 0502 曾据此写出一个版本）。正解：自建 `TcpStream` 时显式 `set_nodelay(true)`
    —— 而"自建 TCP"正是跳板所需的形状（底层流由调用方提供）。

121. **服务端的 `Handler::data()` 对**所有**通道都会被调用**（上游把数据**同时**交给通道自身的接收端
    **与** handler，`server/encrypted.rs:1251`）。因此测试服务端的"回显"必须只对 **shell** 通道执行
    —— 否则 `direct-tcpip` 通道上的字节会被原样回送，客户端读到的是**自己刚写入的 SSH id 行**，
    报错为 `Bad packet size: 1397966893`（该数字即 `"SSH-"`）。
    结论：**假服务端的每个回调都要明确它对哪些通道生效**。

122. **`copy_bidirectional` 出错时不会交出已搬运的字节数**，而收尾时报错是常态 —— 把计数建立在
    它的返回值上，会让"搬运了多少字节"这条判据在最需要时恒为 0。正解：包一层
    `AsyncWrite` 计数写入（同时使该值**在会话仍开启时即可读出**）。

123. **`rusqlite` 不是 app 的 dev-dependency**（`akasha-store` 才是）—— 集成测试中需要书写
    `Connection` 类型时，使用 `akasha_store::Connection` 这一**再导出**，不要在 `Cargo.toml` 中
    新增一份需要版本对齐的重复依赖。

124. **全部 E2E 目标运行在同一个 app 进程中，而内存凭据缓存的键是 `(host, port, user, 认证方式)`**
    （ADR-0003 D8，`Arc` 共享）。因此两个 E2E 目标若使用**同一个 `host:port`**（例如都以
    `akasha-e2e-inner.invalid:22` 作为"仅跳板可见的目标"），而后一个目标提供了**另一条口令**，
    第二个目标会**静默使用缓存中的口令**：服务端拒绝 → `password_step` 仅执行 `forget`、该连接
    随即结束 → 整条认证**失败**，用户（与用例）**不会**被重新询问。症状是"提示问答未走完即无法
    连接"，而 `wait_connected` 的诊断会给出真正原因（`认证失败：… 上没有可用的方式`）。
    正解：**每个 SSH E2E 目标使用自己的目标名**（`akasha-e2e-import.invalid` 即由此而来）——
    端口冲突无影响（跳板端口每次随机），**名字会**。

125. **`thiserror` 的 `#[error("…", expr)]` 不接受位置参数** —— 写 `… 有 {} 处 …", problems.len()`
    会得到 `expected an expression`。要么使用字段引用（`{problems}`，但要求该字段实现 `Display`），
    要么**在构造处拼接完整句子**并存入 `message` 字段（`ImportError::Refused` 即如此）。

126. **登记一个实体时的"初始状态"不是一次状态转移**：plan 0601 最初在 `tunnel_open` 里对新登记的
    隧道再走一次状态机（`→ 连接中`），而它登记时**已经**是那个状态 —— `connecting → connecting`
    被状态机（正确地）判为非法边，整条命令随即失败，表现为"点了打开、界面立刻报状态转移被拒"。
    正解：**在登记处发那条事件**，状态机只管"之后的变化"。

127. **`target_host` / `target_port` 在 plan 0601 不参与连接**：隧道只连**规则所属主机**
    （`forwards.host_id`）。因此"连接失败"的构造点在**主机**那一层 —— 把目标端口写成不可达端口
    不会让它失败（这一版根本不连目标），表现为"用例以为在验失败路径，实际验的是成功路径"。

128. **测试服务端原先只有通道级的断开计数**（`sessions_closed`，由 `channel_close` 回调 +1）：
    隧道**没有通道**（ADR-0003 D4），于是"停下来之后连接真的断了"在这条路上**没有任何观察点**，
    只能得到一个永真的断言。正解：新增**连接级**计数（`Observed::connections_closed`，
    在 handler 的 `Drop` 里数 —— 真实 handler 由 `Server::new_client` 标出，
    `Clone` 出的中间副本不计）。

129. **进程级的 `---p` 判据会被"别人的守卫页"搅动**：`credential_protection` 按
     `/proc/self/maps` 里 `---p` 映射的**总量**判断"8 条凭据 → 多 8 页"，而同一个进程里并发的
     那个测试线程结束时，它的**线程栈守卫页**（同样是 `---p`）会被解除映射 —— 实测 8 页只数到
     7 页（+28 kB 而不是 +32 kB），约一半概率红。
     ⚠️ **它只在 `cargo test` 下发作**（一个二进制里的多个测试运行在同一进程的多个线程上）；
     `just test` 用 **nextest**（一个测试一个进程）因此不触发 —— 门禁不受影响，这也是它长期没被发现的原因。
     正确的做法是**按归属**而不是按总量：`akasha-store` 的用例已经在用 `smaps` 的 `VmFlags`
     （`memsafe` 的页带 `dd` / `wf`，守卫页没有），把这条判据改成同一口径即可。

130. **E2E 共用同一个 app，而"面板"是有状态的浮层**：`.tab-new-tunnel` 是**切换**，面板的规则表
     又是**挂载时读一次**（`TunnelPanel`）—— 于是下一个目标点同一按钮会把上一个目标留下的面板
     **关闭**，表现为"面板列不出规则"（超时），而不是"按钮点错了"。
     正解：`support::open_tunnel_panel` —— **先卸下再挂上**（既幂等，又保证重新读一次池子）。
     教训：共用 app 的 E2E 目标里，"打开某个浮层"必须是幂等的，且要假设它已经是打开状态。

131. **"地址不合规"与"地址没填"是两件事，检查顺序决定报哪一句**：plan 0603 的回环检查
      （SOCKS5 不许绑非回环地址）最初排在"空绑定地址"之前 —— 于是规则里没填绑定地址时，
      用户看到的是一句带着**空地址**的「SOCKS5 监听不能绑到 ：…」。正解：**先判空、再判合规**，
      而且那句"该填什么"要按入站类型分开（`-L` 可以提 `0.0.0.0`，SOCKS5 提它等于推荐一个
      下一句就被拒的地址）。

132. **入站通道的回调运行在连接的消息循环上**：`russh` 把
     `Handler::server_channel_open_forwarded_tcpip` 的 future **在连接的消息循环里 `await`**
     （`client/encrypted.rs`）。因此在这个回调里做任何耗时的 `await`（例如连一个不可达的本机
     地址）都会让**整条连接**无响应 —— 保活也停，而外在表现是"隧道看起来还活着"。
     正解：回调里只做同步的派发（把通道与接受句柄一起送进 mpsc），接线在**任务**里做。
     教训：**回调的 `async` 不等于可以慢** —— 要看它被谁 `await`。

133. **匹配用的键要取"双方都认同的那个"**：`-R` 的入站通道带两个身份 ——
     服务端回报的 `connected_address` 与 `connected_port`。前者**由服务端决定**
     （它认为在听的地址；真实 `sshd` 会受 `GatewayPorts` 一类设置影响，请求 `localhost`
     可能回报 `127.0.0.1`），只有**端口**是我们请求过、服务端回报回来的那一个。
     按地址字符串匹配会在真实服务端上**静默失配**（通道被当成"没登记过"而拒绝），
     而测试服务端原样回报请求里的地址，所以它**测不出来**。
     教训：测试替身"原样回报"的字段，在真实对端那里可能被改写 —— 判据要落在双方都认同的字段上。

134. **`tcpip_forward` 的返回值有两种含义，必须按"请求的是什么"来解**：RFC 4254 §7.1 规定服务端
     **只在请求的就是 0 端口时**才在回复里带端口；请求了具体端口时回复**没有**这个字段。
     上游 `russh` 把"没有字段"表示成 `0`（客户端 `client/encrypted.rs`：*If a specific port
     was requested, the reply has no data* → `Some(0)`；服务端那一侧同样只在 `port == 0` 时才写
     该字段）。于是 `Handle::tcpip_forward(地址, 8080)` 返回的 `0` 意思是**"回复里没有端口"**，
     不是"绑到了 0 端口"。plan 0604 最初直接采用返回值 —— 症状是 probe 报 `127.0.0.1:0`，
     而且停止时拿 0 去 `cancel-tcpip-forward`，**那个监听根本不会被撤掉**（服务端按
     `(地址, 端口)` 查，查不到就回失败）。正解：请求非 0 时端口就是请求的那个；请求 0 时
     必须用回复里的值，它也是 0 就报错。⚠️ 它只在真实路径上暴露 —— crate 用例当时只用 0 端口
     （由服务端挑）验过，**具体端口那条路没有人断言过返回值**。
     （由服务端挑）验过，**具体端口那条路没有人断言过返回值**。（plan 0605 补上了那条断言。）

135. **"状态变了"与"表变了"是两件事，而托盘只订阅了后者**：托盘菜单是一份**快照**
     （`tray::refresh` 挂在 `Sessions::on_change` 上重建整份菜单），而 `Sessions::set_tunnel_state`
     起初**只**往注册表发事件、没有调用 `notify_changed` —— 于是菜单永远停在隧道刚登记时那一行
     （`连接中`），`scope.md` §5.2 指定的"失败必须可见"落点是**死的**（plan 0301 建好了那条管线，
     但状态变化从没通知过它）。正解：状态变化也重推菜单。教训：**一个订阅者 + 快照式重建**的组合里，
     "哪几件事算变化"要逐条核对 —— 少一条不会报错，只会让界面长期显示一个旧值。

136. **收尾信号要带原因，因为触发收尾的地方不止一个**：一条转发的结束有两种来路 ——
     `shutdown()`（我们让它停的）与那条 SSH 连接没了。只报"结束了"的话，重连循环会把用户刚停掉的
     隧道**重新拉起来**。正解：`ForwardEnd { Stopped, ConnectionLost }` 跟着结束信号一起送出去，
     并且循环**再加一道状态判据**（只有实体仍在 `已连接` 才重连）兜住竞争。

137. **`abort()` 杀不掉"任务里又 spawn 的那一层"**：`russh::server::run_stream` 内部把真正的会话
     **又 spawn 了一层**（`session.run(...)`，返回值 `RunningSession` 只是它的包装），abort 外层
     只是丢掉包装 —— 里面的会话照常运行，socket 与 handler 都归它。实测症状：用 abort 实现的
     `cut_connections` 让 `connections_closed` **恒为 0**、"切掉"的连接毫发无损，用例看起来在切、
     实际什么都没切。正解：走会话自己的 `Handle::disconnect`（那也正是"服务端断开这条连接"的真实
     形态）。教训：**abort 的语义要按"那个任务里到底有什么"核对**，包装层的句柄不等于里面那条。

138. **测试替身要对它扮演的东西的生命周期负责**：测试服务端的远端监听原先放在一张**全局**表里、
     只有 `cancel_tcpip_forward` 才会停它 —— 于是"连接断了、监听还在"，而重连对同一个端口的
     `tcpip_forward` 会被**自己上一次留下的监听**顶掉（表现为"重连必然第一次失败"）。真实的 `sshd`
     里转发属于**那条连接**，连接一断端口就还回去。正解：表挂在每条连接的 handler 上，
     `Drop` 时把它的监听一起停掉。

139. **代理里包一层就会多一层生命周期**：`Running` 里存的是包装任务的句柄，而"连接还活着"这件事
     要看**会话**（`Connection::session`）。清点"谁还活着"时按**包装**清点会漏 —— plan 0605 的用法是
     "切连接走会话句柄、`is_finished` 只看包装"，两者各自回答一个不同的问题（问题 #137 的近亲）。

140. **失败分档要按"哪一层坏了"分，而不是按错误类型分**（D13 的落地）：`Connect` / `Jump`（网络）
     与"端口没拿到"（资源被占）应当重试；认证、主机密钥、配置与内部状态**不重试**。⚠️ 分错档的
     后果不对称：该重试的没重试只是少一次自动恢复，而**不该重试的却重试了**会以错误的口令连试三次
     —— 那正是账号锁定的经典成因。它写成 `TunnelError::retryable` 并有两组正反例钉住。

141. **上游只给了同步的"连接还在吗"**：`russh` 的 `client::Handle::is_closed()` 是同步的
     （背后是"会话消息循环的接收端还在不在"），**没有可 `await` 的关闭信号**。因此重连的"断开"靠
     转发任务按固定间隔看一眼（plan 0605 取 500 ms）。⚠️ 它对"半死"（TCP 没断、对端不回话）不敏感
     —— 那种情况要等保活耗尽（`keepalive_interval × keepalive_max`，默认约 90 秒），
     所以"拔网线"在真实网络上的发现延迟是**保活量级**，不是秒级。

142. **在阻塞线程上执行的那件事，丢掉 `await` 那一侧取消不了它**（plan 0606）：`spawn_sync`
     （= `spawn_blocking`）里的握手会一直执行到底 —— 扔掉 `JoinHandle` 只是不再等它，
     它建起来的那个 socket 也一直开着（最长一个 `connect_timeout`，D15 的 10 s）。
     表现是"关闭一条正在握手的隧道之后，对端几秒内仍看得到那条连接"。
     **判据**：凡是要能"当场停"的阻塞活儿，都得把取消信号送进**那次调用自己**（这里做成了
     `SshConnection::connect_via_until(…, cancel)`），而不是指望取消 `await`。

143. **`oneshot` 的发送端换一个，接收端会立刻醒** —— 于是 `select!` 会**随机**挑分支
     （plan 0606 的返工）：一个任务在开始前"登记自己的停止入口"，成功的连接由**看护任务**
     再登记一份、把尝试那一份替换掉；那一刻尝试的 `select!` 两个分支同时就绪，
     `tokio::select!` 随机挑一个 —— 一次**成功**的连接因此有约一半的机会被报成"已停止"。
     改为"**实体自己持有一对 `watch`**、每个动作订一份接收端"之后，接收端只会在真的被要求停止
     （或实体没了）时醒。教训：**信号的所有权要跟着被停止的东西，而不是跟着停止它的那一次动作**。
     ⚠️ 附带一条 `watch` 语义：`Sender::send` 在**没有接收端**时返回 `Err` 且**什么都不做**，
     有接收端时即使值没变也会通知 —— 后者正是"第二次停止照样有效"依赖的性质。

144. **界面显示的那行状态，在一次命令在途期间是**上一次**的**（plan 0703）：点「连接」之后
     后端立刻把那一侧置成 `connecting`（`prepare_connect` 在任何 I/O 之前），而面板要等命令
     返回才刷新 —— 于是**重新连接**期间界面上仍写着"已连接"，看起来那一栏还是可用的。
     两处处置：① 面板在命令在途时按后端的事实显示 `connecting`（这正是后端此刻的状态，
     不是界面猜的）；② **E2E 的完成条件改读 `sftp` 探针**（`state = connected` 且
     `origin.id` 是这一次选的那台）。教训：**"状态"这类断言要读后端的事实，界面上的字可能是
     上一次留下的** —— 这条在一栏重新连接时才暴露，第一次连接时旧值恰好也是"未连接"，所以看不出来。
     ⚠️ 同一个用例在**第二次整份执行**时又红了一次，原因是这条教训的**另一半**：判据改读后端
     之后就**不能紧接着读界面**（面板要等命令返回之后才刷新）—— 那一次读到的是还没渲染的空串。
     正解是**先等那一行出现、再断言它的内容**（`wait_js` + 读文本）：后端的事实可以立刻断言，
     界面的呈现要先等到它出现。

145. **汇总类文档里"像量化收益"的一句话，会在实现时被证伪**（plan 0703）：`scope.md` §4.1
     原写"B 档使本机带宽减半（1×）"、A 档"2×"，而协议层面两档都要**读源一遍、写目标一遍**
     —— 字节数相同，B 档真正的收益是**可直达性**（本机只需够得着 A 一台）。它此前没有出处、
     也没有实测，按字面读会让人去优化一个不存在的一半带宽。处置：`scope.md` 那一格改成
     "本机的可直达性要求"，ADR-0006 D5 记下实测口径。教训：**收益要写成可被证伪的量**，
     写"减半"就得同时写下它的分子分母。

146. **`SshConnection::over` 不持有承载它的那条连接**（plan 0703）：它只造下一跳，
     `under` 是空的 —— 链的存活归调用方（`chain` / `connect_via` 会把整条链放进去）。
     单独用 `over` 建一跳看起来像一条能独立存在的连接，而它的"网络"随时会随承载者消失。
     app 那条路走的是 `connect_via_until`，所以不受影响；这条写下来是因为它是**接口上的一处陷阱**：
     名字相同、语义相邻的两个入口，活着的条件不一样。

147. **测试替身没实现的协议标志，会让判据静默失去对象**（plan 0704）：`testing.rs` 的 SFTP
     服务端把 `OpenFlags::EXCLUDE` 与 `CREATE` 合在一个分支里 `create(true)` —— 于是**它不拒绝**
     第二个同名文件，"两个同名文件不能撞在同一个临时名上"这条判据在库内看起来成立、实际没有
     （两条并发传输会交错写同一个临时文件，而用例全绿）。修法照上游：`EXCLUDE` 走 `create_new`
     （`O_EXCL`）。教训：**替身要不要实现某个标志，是"这条判据还算不算数"的问题，不是脚手架细节**；
     凡是判据依赖的协议语义，替身必须实现它，否则用例测的是替身而不是被测代码。

148. **带时延链路的两个方向接反，报出来的是"握手坏了"**（plan 0704）：把"客户端读"接到
     "客户端写"上，客户端收到的就是自己刚发出去的字节，`russh` 报 `Key exchange init failed`
     —— 那读起来像 KEX 实现有问题，实际上是测试脚手架接错了两根线。同一处还有第二条：
     时延要按"段各自计时"实现，写成"读一段、睡一段、写一段"的循环会让链路自己变成一个
     串行瓶颈，于是并发发出去的请求**在链路里排队**，"并发更快"在上限 2、4 上几乎量不出来。
    教训：**测量装置本身要先被怀疑一次** —— 判据给出反直觉的数字时，先问"这个数是不是装置造出来的"。

149. **`akasha-pty` 曾用 Windows 上不存在的 `rustix::process`，且没有 `cfg` 守卫**
     （**编译面已处置**，plan 0108，2026-09-15）：上游把 `rustix::process` 限定在
     `#[cfg(not(windows))]`，而 `teardown.rs` / `watchdog.rs` 直接用它的 `Pid` / `Signal` /
     `kill_process` / `setsid` —— 于是 Windows 目标编译不过（`cargo check --target
     x86_64-pc-windows-msvc` 在 `akasha-pty` 就红，3 个 E0432 / E0433）。它长期没暴露的原因是
     CI 的 Windows 那一格（`checks-other`）在此之前没有执行过（首次运行已由推送触发，见 plan 0102）。
     **现在的边界**：编译不再是障碍，但 Windows 上"回收整个会话"**仍然是空的**（`kill_session`
     返回 0，`Child::kill()` 只收得走 shell 自身）—— 等价物是 Job Object，它要一台 Windows 主机
     才能验收，那条缺口记在「进行中 / 下一步」。⚠️ 阶段 8 那条判据（Windows / macOS 的原生编译）
     在本机仍然只有依赖图核对与不带 C 构建脚本的成员，完整证据在 CI 的 Windows 格子。

150. **枚举出来的端口不保证能打开**（plan 0802 实测）：本机（libudev 那套）列出 32 条
     `/dev/ttyS0`…`/dev/ttyS31`，而 `/dev` 下**一个都没有** —— 上游按 udev 设备给 devnode，
     不检查那个节点在 `/dev` 下是否存在；它那句"打不开就跳过"的过滤
     （`serialport` 的 `enumerate.rs`：parent 的驱动是 `serial8250` 且打不开时跳过）本机没有触发。
     同一处还有第二个读数：`--no-default-features`（sysfs 那套）在**同一台机器**上返回 0 条，
     因为它要求 `/dev/<名字>` 存在。教训：**"列出"不等于"能用"** —— 判据写成"枚举到的端口都能
     打开"会得到一条与真机相反的断言（本机恒假、接上设备恒真）；正确的位置是"枚举只负责列，
     打开失败带路径与原因"（`SerialError::Open`），而界面不要把列表当成"可用端口"。

151. **串口接入 app 在 ROADMAP 里没有条目**（plan 0802 收尾时发现，2026-09-15 由**阶段 11** 处置）：
     阶段 8 的两条都在 crate 层，`scope.md` §2 的"三大终端之一（serial）"当时没有用户可见的形态。
     编号不复用；教训：**一个阶段的判据全在 crate 层时，产品形态的那一半要有自己的条目**。

152. **把多类检查写成配方的多行 = 快速失败会掩盖其余结论**（本会话）：`just docs-check` 原先的第一行是
     `@just docs-style`，而 just 在配方里某一行失败时立即终止该配方 —— 于是语体一命中，
     命令未漂移 / ROADMAP 预算 / plan 预算三类检查**一次都没有执行**，输出里也看不出它们没有执行。
     这与 CI 矩阵用 `fail-fast: false` 是同一个理由：**互相独立的信号不串成一条链**。
     正解：四部分放进同一个 shell 块（变量才能跨检查累加），最后一次性汇总退出。
     教训：配方里"再调用另一个配方"的那一行，等于给整条链加了一个**早退点**。

153. **裸词正则会匹配英文散文**：反向命令检查若把 `CLAUDE.md` 算进去，Victauri 自动生成块里的英文句子
     （`just` 后面紧跟 `retry` 一类单词）会被读成配方名。它不在检查范围内不是因为那个文件不重要，
     而是因为裸词正则只对中文文档成立（口径写在 `docs/just.md` §2）。

154. **`rustc-wrapper` 让"没有 sccache 的环境一个文件都编译不了"**（CI 首次运行的唯一红因）：
     `.cargo/config.toml` 里 `rustc-wrapper = "sccache"`，而镜像上没有 sccache —— cargo 在**探测
     rustc** 时就失败，报错读起来像工具链坏了，与真正的编译错误不是同一种。两个可选处置：
     **装 sccache**（`taiki-e/install-action` 的 `TOOLS.md` 三平台都收录）或**去掉那层包装**。
     选后者的两条理由：`Swatinem/rust-cache` v2.7.8 的 README 逐项列出它缓存的目录，只有
     `~/.cargo` 与 `./target`，**不含** sccache 自己的缓存目录 —— 那层包装在 CI 上不可能命中；
     且空值即"没有包装"（本机 cargo 1.98.1 实测：把文件里的包装器换成不存在的那个，只要
     `RUSTC_WRAPPER` 为空就照样通过，证明空值真的覆盖了配置）。
    ⚠️ **2026-09-17 起 CI 那一侧只是兜底**：仓库里的包装器已整份删除（问题 #163），
    因为同一个包装器在 mise 的 cwd 解析下同样让依赖编不过 —— 两处处置至此收敛到同一条口径。

155. **Tauri 的官方依赖列表里没有 `libudev-dev`，而 `serialport` 需要它**（CI 第二次运行）：
     Linux 那一格红在 `just ready` → `lint` → `clippy`，报的是 `libudev-sys v0.1.4` 的构建脚本
     panic —— `pkg-config --libs --cflags libudev` 答 `Package 'libudev' … not found`。
     ⚠️ 缺这个**开发包**与"运行期没有 libudev"是两件事：后者降级成 sysfs 那套枚举（空表，不是
     错误 —— 见阶段 8 那一行），前者是**编译不过**。处置：加进 `env.APT_DEPS`。

156. **Windows 上 `perl` 解析到 Git 自带的 msys 版本，而 vendored OpenSSL 需要 Strawberry Perl**
     （CI 第二次运行）：`openssl-sys` 的构建脚本执行 OpenSSL 的 `Configure`，报
     `Can't locate Locale/Maketext/Simple.pm in @INC`，而 `@INC` 全是 `/usr/share/perl5/core_perl/...`
     —— 那是 Git for Windows 的 perl；镜像另装了 Strawberry Perl（chocolatey 的 `strawberryperl`），
     只是 PATH 里排在后面。处置：`OPENSSL_SRC_PERL` 指向 Strawberry 的 `perl.exe`
     （`.github/actions/windows-perl`，`checks-other` 与 `e2e` 共用）。⚠️ 写进 `GITHUB_ENV` 之前
     先探一次模块：失败因此停在**那一步**，而不是推迟到 cargo 的构建脚本里 —— 后者的报错读起来
     像 OpenSSL 坏了，与真正的原因不是同一种。

157. **`Swatinem/rust-cache` 默认在仓库根执行 `cargo metadata`，而本仓库的 workspace 在 `src-tauri/`**：
     每个执行到它的 job 都会在缓存步骤里打一串 `Error: The process … cargo … failed with exit code 101`
     与 `could not find Cargo.toml in …`（该步骤仍判**成功**，键也照常给出 —— 所以它不会挡住任何
     东西，只会让缓存范围无法从日志判断）。它是 ADR-0004 的直接后果：该 action 的 `workspaces`
     默认值是 `. -> target`。处置：三处都补 `workspaces: src-tauri`。

158. **用例中途失败会把标签页留在 app 上，后面的目标因此必红**（CI 第三次运行，macOS 的 E2E）：
     macOS 上三条串口目标打不开 PTY 从端（问题 #160 那一类平台限制），失败发生在**开标签页之后**
     —— 那三个标签页留在了 app 上，于是下一个目标 `bw_import` 的 `is_connected`（它要求标签页数
     **恰好**等于脚本里写的那个数）永远不成立，报出来的是"提示问答没走完就连不上"，而状态栏明明
     写着"已连接" —— 两句话看起来像产品缺陷，实际是上一个目标留下的污染。教训：**判据里数总量时，
     失败路径的残留就是它的污染源**；平台跳过要发生在**动界面之前**，而不是等失败了再补救。

159. **bash 会把紧跟在 `$变量` 后面的全角标点读进变量名**（CI 第三次运行，macOS 的 E2E 收尾）：
     macOS 的 bash 3.2 在那里把 `$l）` 解析成变量名 `l` + 半个多字节序列，`set -u` 于是报
     `l：unbound variable` 并以 **127** 退出 —— 而这一行只在**已经失败**时才执行，于是它把真正的
     失败换成了"命令找不到"。同一行在 Linux 的 bash 5 上照常工作，所以它只在 macOS 上暴露。
     处置：`$变量` 紧邻非 ASCII 字符时一律写花括号形式（本轮扫了全部 justfile 与测试脚手架，
     7 处一并改掉）。

160. **`portable-pty` 的 `tty_name` 只在 Unix 上有，而测试脚手架直接用了它**（CI 第三次运行，
     Windows 的类型检查）：`error[E0599]: no method named tty_name found for struct Box<(dyn
     MasterPty + Send + 'static)>` —— 测试要的是"从端的设备名"，而 ConPTY 没有设备节点这个概念。
     它与问题 #149 同类（平台专有 API 漏了 `cfg`），只是这一处落在**测试**里，而 `akasha` 带 C 依赖、
     本机无法为 Windows 目标构建 ⇒ 只有 CI 的 Windows 格子能给出读数。处置：`slave_device_name`
     按 `cfg(unix)` 分两条实现，三条串口 E2E 在非 Linux 平台上按 `fake_serial_skip_reason` 显式跳过。

161. **`main` 上的运行是排队等待，看起来像"卡住"**（本会话实测）：`concurrency` 的规则是
     "同一分支只保留最新一次运行，`main` 除外" —— 于是前一次还没结束时，后一次的状态**一直是**
     `pending`；一个分组里最多留一个排队中的运行，**取消正在执行的那次会把排在它后面的那次一起
     结束**（实测两次运行同时变成 `cancelled`）。教训：先分清"等待"与"卡住" —— 判据是
     `pending` 且前一次仍在 `in_progress`、以及各 job 的典型耗时，而不是运行时长本身。
     处置：workflow 补 `workflow_dispatch`（重新验证某次提交不必加空提交）、排队语义与耗时表
     写进 `docs/just.md` §8。

162. **Windows 上 `just test-e2e` 起不来 app，而那份 app 日志是空的**（CI 第三次运行，第一次读数）：
     那格在 **13 分 38 秒**的构建之后执行到"起 app"，120 秒内没有任何 app 登记进 discovery 目录，
     配方于是报 `❌ app 没起来`，而它 `tail` 的那份日志**一个字节都没有**；随后那一次运行被手动
     停止（收尾处的退出码记成 `0xC000013A`，那是控制台被中断的形状，不是 app 自己的退出码）。
     ⚠️ **没有定位**：本机没有 Windows 主机，"进程有没有起来、走到哪里才没有的"都观测不到。
     下一轮要抓的三样证据已经写进配方（日志为空时会明说、cargo 进程还在时会明说）：`cargo run`
     自己的退出码、`tasklist` 里有没有 `akasha.exe`、discovery 目录有没有出现过。
     在那之前 Windows 的 E2E 格子仍然是红的。

 163. **同一个 `rustc-wrapper` 在本机 macOS 上让每一个依赖都编不过**（本会话实测，macOS 26.6.2 /
      arm64）：`.cargo/config.toml` 把 rustc 包成 `sccache`，而本机的 sccache 由 mise 提供 ——
      mise 的 shim 按**当前目录**解析版本，cargo 编译 registry 里的依赖时 cwd 落在
      `~/.cargo/registry/src/<crate>-<版本>/`，那里解析不到版本，于是**每一条** rustc 调用都以
      `mise ERROR No version is set for shim: sccache` 失败，`just dev` / `just check` / `just test`
      一个文件都编不出来（报错读起来像编译器坏了）。**不经 cargo 也能复现**：
      `cd ~/.cargo/registry/src/index.crates.io-*/serde_core-* && sccache --version`。
      它与问题 #154 同因（配置里的包装器在"包装器不可用"的环境里一票否决），触发条件从
      "没装 sccache"变成"装了、但按 cwd 解析不到"。
      处置：**删除 `.cargo/config.toml`** —— 与 #154 里 CI 的选择（清空 `RUSTC_WRAPPER`）收敛到
      同一个口径：包装器不再进仓库配置；需要缓存时另行安装 sccache，再显式 `RUSTC_WRAPPER=sccache just check`。
      ⚠️ 沙箱里直接执行 `just dev` 另有一处环境限制（PTY 报权限不足），与本条无关；用户终端下正常。

 164. **macOS 上被 SIGKILL 的子进程会停在"正在退出"上，直到主端被关闭**（本会话实测，
      macOS 26.6.2 / arm64）：`kill_session` 发出 SIGKILL 之后，`/bin/sh` 在 `ps` 里显示
      `?Es`（`E` = trying to exit、`s` = session leader、命令名已带括号），而 `wait4` 一直
      不返回 —— 同一时刻 `kill(pid, 0)` 仍然成功。**这一步是关键**：`wait4` 阻塞的原因不是
      "信号没送到"（再发一次 SIGKILL 也无效），而是子进程的退出要等终端那一路收干净，而主端
      还开着、又没人读。处置：`PtyTransport::shutdown` 在 `wait` **之前**主动关闭写端 / 读端 /
      主端（`master` 因此变成 `Option`）。⚠️ 这不是清理动作而是**结束条件** ——
      少了它，关标签页会无限阻塞（`just test` 里两条最普通的 shutdown 用例就是这么红的）。
      判别口径：Linux 上同样的顺序不出这个问题，所以它只在 macOS 的读数上现形。

 176. **Windows 上仍有两类 E2E 目标被显式跳过，各自缺的东西不同**（本会话实测）：修掉 #162 / #173 / #174
      与上面那两条平台差异之后，剩下的红灯一律改成显式跳过并在日志里写明原因（`AGENTS.md` §7）——
      - `tab_close` / `window_close`：探针是一个 **POSIX shell 程序**（`sh -c 'trap "" HUP; …' &`），
        Windows 的默认 shell 是 cmd.exe，写不出"忽略 SIGHUP 的后台作业"；而"关标签页 / 关窗把它一起收走"
        在那边要靠**作业对象（Job Object）**，尚未实现（plan 0108 留下的缺口）。
      - `bitwarden_login` / `bw_import`：假 `bw` 是一份 `#!/bin/sh` 脚本，Windows 上 `CreateProcess` 不执行
        脚本 —— 需要一个真的 `.exe`，本仓库还没有（`install_fake_bw` 在那边写出来的文件名已经是 `bw.exe`）。
      ⚠️ 这两类都**不是"执行不了"，是判据本身在那边还不成立**；显式跳过只是把这件事说清楚，缺口仍在。


 181. **macOS 上"真正退出零残留"的判据是空过的，而状态文件把它读成了通过**（2026-09-27 覆盖审计
      发现，**未修**，plan 0112）：`tests/exit_residue.rs:92` 的 `alive()` 读 `/proc/<pid>` 且
      **没有平台门控** —— macOS 上没有 `/proc`，它恒为 `false`，于是"关窗后 app 真的退出"那条断言
      立刻为真；`:189` 又在非 Linux 不启动探针。CI 的 `E2E（macOS）` 日志里这一段
      `1 passed ... finished in 0.22s`（真的走完那条路径要 30 s 级窗口）——**这就是空过的形状**。
      同一类还有 `tests/session_watchdog.rs:170`：跳过理由写"macOS 要 `proc_listpids`"，
      而 `pty/teardown.rs` 的 `ps` + `getsid` 实现早已落地（那条理由已过期，macOS 上因此少一条
      端到端判据）。⚠️ 这类失效**不会变红**，只会让"验过了"变成一句没有证据的话 —— 处置与负例见 plan 0112。


 182. **macOS 的便携数据目录：文档写的是 `.app` 旁边，实现落在 `.app` 内部**（2026-09-27 覆盖审计
      发现，**按口径变更收口**，plan 0408）：`docs/portable.md:32` 与 `docs/scope.md:583` 承诺
      macOS 的数据目录在 `.app` **旁边**，而 `config/ipc.rs` 的 `exe_dir()` 取
      `current_exe().parent()` —— 打包之后那是 `Foo.app/Contents/MacOS`，标记目录放不进去
      （不可写、且破坏签名），放到 `.app` 旁边又推导不到。`tests/portable.rs` 用的是**裸二进制**
      复制进临时目录的布局，所以这条差异在 CI 全绿的情况下也不可见。
      口径裁定（用户）：**macOS 不做便携**（安装形态是 dmg、数据取 OS 标准目录），
      便携只保留 Windows / Linux；Linux 的分发形态**暂定**（可能改用 AppImage，
      其可执行文件位于只读挂载点内，判据的等价物尚未定）。


 183. **macOS 上 SSH 目标的"回声"判据稳定超时**（2026-09-27，本机发现，**未定位**）：
      `ssh_session` / `ssh_jump` / `ssh_config_import` 这几条用例把 SSH 会话连起来之后（界面报
      "已连接"）敲一行命令，屏幕在 30 s 内始终等不到远端回声，用例在 `tests/support/mod.rs` 的
      `wait_js` 上超时（四次运行里每次变红的目标不全相同：两次是 `ssh_session` +
      `ssh_config_import`，一次是 `ssh_session` + `ssh_jump`）。app 日志里 `ssh session opening`
      与 `ssh authenticated` 都在，会话之后被正常回收 —— 问题落在"键入的字节到不到对端、对端的
      字节回不回终端"这一段。
      ⚠️ **与 plan 0112 / 0408 无关，已用原始工作区验证**：把 plan 0112 的全部改动 `git stash`
      之后重新执行 `just test-e2e`，同一断言、同样超时。它拦住的是 `just test-e2e` 的整体退出码 0
      （其余目标与三段判据都过）。
