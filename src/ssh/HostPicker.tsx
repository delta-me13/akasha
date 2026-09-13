// 主机选择器 —— "从界面选一个主机"（plan 0504）。
//
// 它是**壳层**：列出池里已有的行，选一行交给 `onConnect`。没有新建 / 编辑 / 删除
// —— 那是仍未规划的界面工作（本步只把库里已有的东西列出来）。
//
// ⚠️ 库锁着时列不出来（池在库里）：这里把那句话原样显示，**不假装列表是空的**
// —— "一台主机都没有"与"读不出来"是两件事，混在一起会让用户去建一台已经存在的机器。

import { useCallback, useEffect, useState } from "react";
import { HostsUnavailable, listHosts, type HostEntry } from "../ipc/hosts";

interface HostPickerProps {
  onConnect(host: HostEntry): void;
  onClose(): void;
}

/** 跳板那一行的名字（池里找不到就退回 id —— 池是唯一真相源，界面上不该编一个名字）。 */
function nameOf(hosts: HostEntry[], id: number): string {
  return hosts.find((host) => host.id === id)?.name ?? `#${id}`;
}

export function HostPicker({ onConnect, onClose }: HostPickerProps) {
  const [hosts, setHosts] = useState<HostEntry[] | null>(null);
  const [problem, setProblem] = useState<string | null>(null);

  const reload = useCallback(() => {
    setProblem(null);
    void listHosts()
      .then(setHosts)
      .catch((err: unknown) => {
        setHosts(null);
        setProblem(err instanceof HostsUnavailable ? err.message : String(err));
      });
  }, []);

  useEffect(reload, [reload]);

  return (
    <div className="host-picker" role="dialog" aria-label="选择 SSH 主机">
      <p className="host-picker-title">选一台主机打开 SSH 会话</p>
      {problem && (
        <p className="host-picker-problem" role="alert" data-picker-problem>
          {problem}
        </p>
      )}
      {hosts && hosts.length === 0 && (
        <p className="host-picker-empty">
          主机池里还没有东西。池的增删改查还没有界面（plan 0403 落的是库那一侧）。
        </p>
      )}
      {hosts && hosts.length > 0 && (
        <ul className="host-picker-list">
          {hosts.map((host) => (
            <li key={host.id}>
              <button
                type="button"
                className="host-picker-item"
                data-host-id={host.id}
                onClick={() => onConnect(host)}
              >
                {host.name}
                <span className="host-picker-target">
                  {host.user}@{host.host}:{host.port}
                </span>
                {host.jumpId !== null && (
                  // 跳板（plan 0505）：这一次点下去会**经过谁**。链的其余部分由后端走，
                  // 这里只显示一跳 —— 界面上有这一笔，连不上时才分得清是目标还是跳板的问题。
                  <span className="host-picker-jump" data-host-jump={host.jumpId}>
                    经跳板 {nameOf(hosts, host.jumpId)}
                  </span>
                )}
              </button>
            </li>
          ))}
        </ul>
      )}
      <div className="host-picker-actions">
        <button type="button" className="host-picker-close" onClick={onClose}>
          取消
        </button>
      </div>
    </div>
  );
}
