// 串口选择器 —— "从界面选一条串口配置"（plan 1101）。
//
// 它是**壳层**：列出池里已有的行，选一行交给 `onConnect`。没有新建 / 编辑 / 删除
// —— 那是仍未规划的界面工作（串口池的 CRUD 界面与主机池的那个缺口是同一批）。
//
// ⚠️ 库锁着时列不出来（池在库里）：这里把那句话原样显示，**不假装列表是空的**
// —— "一条配置都没有"与"读不出来"是两件事（同 `HostPicker`）。
//
// ⚠️ 计划到 plan 1102 才有的东西**不在本条**：端口枚举（`ports()`）、手动输入路径、
// 参数可编辑与越界提示。本条只走"从池里挑一行"这一条输入路径。

import { useCallback, useEffect, useState } from "react";
import { SerialsUnavailable, listSerials, type SerialEntry } from "../ipc/serials";

interface SerialPickerProps {
  onConnect(serial: SerialEntry): void;
  onClose(): void;
}

/** 校验位在 "8N1" 里的那个字母（与设备的通称一致，而不是库里的列值）。 */
function parityLetter(entry: SerialEntry): string {
  switch (entry.parity) {
    case "none":
      return "N";
    case "even":
      return "E";
    case "odd":
      return "O";
  }
}

/** 一行配置的摘要：`/dev/ttyUSB0 · 115200 8N1`。 */
function framing(entry: SerialEntry): string {
  return `${entry.port} · ${entry.baud} ${entry.dataBits}${parityLetter(entry)}${entry.stopBits}`;
}

export function SerialPicker({ onConnect, onClose }: SerialPickerProps) {
  const [serials, setSerials] = useState<SerialEntry[] | null>(null);
  const [problem, setProblem] = useState<string | null>(null);

  const reload = useCallback(() => {
    setProblem(null);
    void listSerials()
      .then(setSerials)
      .catch((err: unknown) => {
        setSerials(null);
        setProblem(err instanceof SerialsUnavailable ? err.message : String(err));
      });
  }, []);

  useEffect(reload, [reload]);

  return (
    <div className="serial-picker" role="dialog" aria-label="选择串口配置">
      <p className="serial-picker-title">选一条串口配置打开会话</p>
      {problem && (
        <p className="serial-picker-problem" role="alert" data-picker-problem>
          {problem}
        </p>
      )}
      {serials && serials.length === 0 && (
        <p className="serial-picker-empty">
          串口配置池里还没有东西。池的增删改查还没有界面（plan 0403 落的是库那一侧）。
        </p>
      )}
      {serials && serials.length > 0 && (
        <ul className="serial-picker-list">
          {serials.map((serial) => (
            <li key={serial.id}>
              <button
                type="button"
                className="serial-picker-item"
                data-serial-id={serial.id}
                onClick={() => onConnect(serial)}
              >
                {serial.name}
                <span className="serial-picker-target">{framing(serial)}</span>
              </button>
            </li>
          ))}
        </ul>
      )}
      <div className="serial-picker-actions">
        <button type="button" className="serial-picker-close" onClick={onClose}>
          取消
        </button>
      </div>
    </div>
  );
}
