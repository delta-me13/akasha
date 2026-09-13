// 主机池的读取，以及**第一条写路径**：从 `~/.ssh/config` 导入（plan 0506）。
//
// 为什么这些命令住在 `src/ipc/`：前端唯一允许碰后端的目录（`AGENTS.md` §0 禁止 #1）。
// 为什么除了"导入"就没有别的写：增删改是**用户动作**，各有各的判据（重名怎么办、跳板成环
// 怎么提示、删掉被引用的行怎么解释），属于仍未规划的界面工作。导入不一样 —— 它没有"填什么"
// 的自由度，只有"照不照这份文件做"这一个问题，而那个问题的答案在 `akasha_store::sshconfig` 里。

import {
  commands,
  type ConfigFinding,
  type HostEntry,
  type ImportError,
  type ImportReport,
  type VaultError,
} from "./bindings";

/** 界面看得见的一台主机（池里的一行）。 */
export type { HostEntry, ImportReport, ConfigFinding };

/** 读主机池失败。 */
export class HostsUnavailable extends Error {
  readonly detail: VaultError;

  constructor(detail: VaultError) {
    super(HostsUnavailable.describe(detail));
    this.name = "HostsUnavailable";
    this.detail = detail;
  }

  /** 库没解锁 —— 界面该说的是"先解锁"，而不是"读不出来"。 */
  get isLocked(): boolean {
    return this.detail.kind === "locked";
  }

  private static describe(detail: VaultError): string {
    if (detail.kind === "locked") return "库是锁着的：先解锁才能列出主机（池在库里）";
    // "库不可用"那几个变体里只有一部分带 `message`（例如 `passphraseTooLong` 带的是 `max`）
    // —— 所以这里按"带不带"取，而不是按 kind 逐个列举：列一遍就会漏一个。
    if ("detail" in detail && "message" in detail.detail) return `库不可用：${detail.detail.message}`;
    return `库不可用：${detail.kind}`;
  }
}

/** 库里主机池的全部行（按名字排序）。库锁着 → [`HostsUnavailable::isLocked`]。 */
export async function listHosts(): Promise<HostEntry[]> {
  const result = await commands.vaultHosts();
  if (result.status === "error") throw new HostsUnavailable(result.error);
  return result.data;
}

// ── 从 `~/.ssh/config` 导入（plan 0506）───────────────────────────────────────

/** 导入失败。**三类要分得开**：锁着 / 读不到那个文件 / 整份不能照着做。 */
export class ConfigImportFailed extends Error {
  readonly detail: ImportError;

  constructor(detail: ImportError) {
    super(ConfigImportFailed.describe(detail));
    this.name = "ConfigImportFailed";
    this.detail = detail;
  }

  /** 库没解锁 —— 导入要写进池，所以这里不是"读不出来"，是"先解锁"。 */
  get isLocked(): boolean {
    return this.detail.kind === "locked";
  }

  /**
   * 整份被拒时的那几处问题（`Match` / `Include` / 不认识的指令）。
   *
   * ⚠️ 这三件事要分得开：`problems` 非空 = "**看见了**这份配置里的哪几行"，而不是
   * "这个文件打不开"。混在一起用户会去查文件权限。空数组 = 不是这一类失败。
   */
  get problems(): ConfigFinding[] {
    return this.detail.kind === "refused" ? this.detail.detail.problems : [];
  }

  private static describe(detail: ImportError): string {
    switch (detail.kind) {
      case "locked":
        return "库是锁着的：导入要把条目写进 ssh 配置池（先解锁）";
      case "unreadable":
        return `读不到 ${detail.detail.path}：${detail.detail.message}`;
      case "refused":
        return detail.detail.message;
      case "failed":
        return `导入失败：${detail.detail.message}`;
    }
  }
}

/**
 * 把一份 `~/.ssh/config` 导入 ssh 配置池。
 *
 * `path` 传 `null` = 后端按 `~/.ssh/config` 推（报告里的 `path` 是**真读的那个文件**）。
 * `overwrite` = `false`：同名已存在就**不动它**，只在报告里列出来。
 */
export async function importSshConfig(
  path: string | null,
  overwrite: boolean,
): Promise<ImportReport> {
  const result = await commands.importSshConfig(path, overwrite);
  if (result.status === "error") throw new ConfigImportFailed(result.error);
  return result.data;
}
