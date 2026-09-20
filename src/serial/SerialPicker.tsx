// 串口面板 —— **三条并列的输入 → 一个会话**（plan 1101 立、plan 1102 成表单）。
//
// | 输入 | 给出什么 |
// |---|---|
// | 本机枚举到的端口（`serial_ports`） | 设备路径（点一条填进那一栏） |
// | 手输的设备路径 | 设备路径 —— **枚举不可用时的兜底** |
// | 库里的配置行（`vault_serials`） | 路径 + 五个参数（点一行填满整张表单） |
//
// ⚠️ **三条输入是并列的，谁都不挡谁**：枚举那一块读不出来时只有它自己显示那句报错，
// 手输那条路照常可用（问题 #150 的处置：列表不等于"可用端口"，所以更不该由它决定别的输入）。
// 点一条枚举结果**只是把路径填进去** —— 这里不标"可用 / 不可用"，也不试开一次。
//
// ⚠️ **取值域不归这一层**（见 [`toParams`]）：数据位填 9 会照原样交给后端，由它报字段与取值
// （`SerialIpcError::Settings`）；前端只拦"IPC 上寄不出去"的那一档（空串 / 非数字 / 超出类型宽度）。
//
// ⚠️ 打开失败的那句话**不在这里显示**：会话是那个标签页自己发起的（`App.tsx` 只负责开一个面，
// 见 `attachTerminal`），所以"串口打不开 / 参数不合法"由标签页的报错行显示 ——
// 与 SSH 那条路一样。这里显示的是**读列表**失败与**填不出参数**这两种"还没开面"的失败。

import { useCallback, useEffect, useState } from "react";
import {
  SerialPortsUnavailable,
  SerialsUnavailable,
  listPorts,
  listSerials,
  type SerialEntry,
  type SerialPort,
} from "../ipc/serials";
import type { SerialParams } from "../ipc/session";

interface SerialPickerProps {
  /** 参数齐了就把这一面交给壳层（由它开一个串口标签页，会话在那个面里发起）。 */
  onConnect(params: SerialParams, title: string): void;
  onClose(): void;
}

/** 表单的六个字段。数值三栏是**文本**：越界取值要能从界面产生，而 `<select>` 里写不出 9。 */
interface Form {
  readonly port: string;
  readonly baud: string;
  readonly dataBits: string;
  readonly stopBits: string;
  readonly parity: SerialParams["parity"];
  readonly flow: SerialParams["flow"];
}

/** 「常用」那一套 = `SerialSettings::new` 的默认（8 数据位 / 1 停止位 / 不校验 / 无流控）。 */
const EMPTY_FORM: Form = {
  port: "",
  baud: "115200",
  dataBits: "8",
  stopBits: "1",
  parity: "none",
  flow: "none",
};

const PARITIES = [
  { value: "none", label: "不校验" },
  { value: "even", label: "偶校验" },
  { value: "odd", label: "奇校验" },
] as const satisfies readonly { value: SerialParams["parity"]; label: string }[];

const FLOWS = [
  { value: "none", label: "无" },
  { value: "software", label: "软件（XON/XOFF）" },
  { value: "hardware", label: "硬件（RTS/CTS）" },
] as const satisfies readonly { value: SerialParams["flow"]; label: string }[];

type Parsed = { readonly params: SerialParams } | { readonly problem: string };

/** 一个整数栏 → 一个数，或者一句"这一栏填不了"。
 *
 * `max` 是 IPC 上那个整数的**宽度**（`u32` / `u8`），不是取值域 —— 数据位填 9 会通过这里，
 * 由后端的 `DataBits::try_from` 报 `data_bits = 9`。
 */
function whole(raw: string, label: string, max: number): number | string {
  const text = raw.trim();
  const value = Number(text);
  if (text === "" || !Number.isInteger(value) || value < 0 || value > max) {
    return `${label}填不了「${raw}」：这一栏在 IPC 上是一个 0 到 ${max} 的整数`;
  }
  return value;
}

/** 表单 → 六个字段。见文件头：**范围**那一档留给后端，这里只保证寄得出去。 */
function toParams(form: Form): Parsed {
  const baud = whole(form.baud, "波特率", 4_294_967_295);
  if (typeof baud === "string") return { problem: baud };
  const dataBits = whole(form.dataBits, "数据位", 255);
  if (typeof dataBits === "string") return { problem: dataBits };
  const stopBits = whole(form.stopBits, "停止位", 255);
  if (typeof stopBits === "string") return { problem: stopBits };
  return {
    params: {
      port: form.port,
      baud,
      dataBits,
      stopBits,
      parity: form.parity,
      flow: form.flow,
    },
  };
}

function fromEntry(entry: SerialEntry): Form {
  return {
    port: entry.port,
    baud: String(entry.baud),
    dataBits: String(entry.dataBits),
    stopBits: String(entry.stopBits),
    parity: entry.parity,
    flow: entry.flow,
  };
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

function hex(value: number): string {
  return value.toString(16).padStart(4, "0");
}

/** 一条端口的类别说成人话。两档的描述字段**都可能缺**（缺了就不显示那一截，不留一句假话）。 */
function describeKind(kind: SerialPort["kind"]): string {
  // ⚠️ 这些不是字符串标签，而是**结构体变体**（生成物里是 `{ usb: {...} } & { windows?: never }`
  // 这类联合）—— 所以先用属性存在与否收窄，不能在 `kind === "pci"` 上比。
  if (typeof kind === "string") {
    if (kind === "pci") return "PCI 串口";
    if (kind === "bluetooth") return "蓝牙串口";
    return "类别未知";
  }
  if (kind.windows) {
    // Windows 专有的一档（plan 0803）：上游在那边报不出 USB / PCI，描述只有注册表给得出。
    // 两项都可能缺 —— 只显示读到的那些，读不到的就不显示。
    const { friendlyName, hardwareId } = kind.windows;
    return [friendlyName, hardwareId].filter((part) => !!part).join(" · ") || "Windows 串口";
  }
  // 剩下这一支就是 `usb`：TS 的收窄让"契约里加一档"在这里变成编译错误。
  const { vid, pid, manufacturer, product } = kind.usb;
  const name = [manufacturer, product].filter((part) => !!part).join(" ") || "USB 串口";
  return `${name} ${hex(vid)}:${hex(pid)}`;
}

export function SerialPicker({ onConnect, onClose }: SerialPickerProps) {
  const [ports, setPorts] = useState<SerialPort[] | null>(null);
  const [portsProblem, setPortsProblem] = useState<string | null>(null);
  const [serials, setSerials] = useState<SerialEntry[] | null>(null);
  const [serialsProblem, setSerialsProblem] = useState<string | null>(null);
  const [form, setForm] = useState<Form>(EMPTY_FORM);
  /** 这张表单是照池里哪一行填的（填完没改过时，标签页标题用那个名字）。 */
  const [source, setSource] = useState<SerialEntry | null>(null);
  /** **还没开面**就失败的那一档（列表读不出来 / 参数填不出来）。 */
  const [problem, setProblem] = useState<string | null>(null);

  // 两条读取**各自独立**：端口枚举挂了不影响池，也不影响手输（文件头的第一条）。
  const loadPorts = useCallback(() => {
    setPortsProblem(null);
    void listPorts()
      .then(setPorts)
      .catch((err: unknown) => {
        setPorts(null);
        setPortsProblem(err instanceof SerialPortsUnavailable ? err.message : String(err));
      });
  }, []);

  const loadSerials = useCallback(() => {
    setSerialsProblem(null);
    void listSerials()
      .then(setSerials)
      .catch((err: unknown) => {
        setSerials(null);
        setSerialsProblem(err instanceof SerialsUnavailable ? err.message : String(err));
      });
  }, []);

  useEffect(loadPorts, [loadPorts]);
  useEffect(loadSerials, [loadSerials]);

  /** 改任何一栏都意味着"这张表单不再等于池里那一行" —— 标题随之退回设备路径。 */
  const edit = (patch: Partial<Form>) => {
    setSource(null);
    setProblem(null);
    setForm((current) => ({ ...current, ...patch }));
  };

  const useEntry = (entry: SerialEntry) => {
    setProblem(null);
    setSource(entry);
    setForm(fromEntry(entry));
  };

  const submit = () => {
    const parsed = toParams(form);
    if ("problem" in parsed) {
      setProblem(parsed.problem);
      return;
    }
    onConnect(parsed.params, source?.name ?? (parsed.params.port || "串口会话"));
  };

  return (
    <div className="serial-picker" role="dialog" aria-label="打开一个串口会话">
      <p className="serial-picker-title">打开一个串口会话：挑一条、或者手输一条</p>

      <section className="serial-ports" data-serial-ports={ports ? ports.length : ""}>
        <p className="serial-ports-title">
          本机枚举到的端口（{ports ? `${ports.length} 条` : "读取中…"}）
        </p>
        {portsProblem && (
          <p className="serial-ports-problem" data-ports-problem role="alert">
            {portsProblem}
          </p>
        )}
        {ports && ports.length === 0 && (
          <p className="serial-ports-empty" data-ports-empty>
            系统认为本机没有串口 —— 下面手输一条路径仍然可用
          </p>
        )}
        {ports && ports.length > 0 && (
          <>
            <ul className="serial-ports-list">
              {ports.map((port) => (
                <li key={port.path}>
                  <button
                    type="button"
                    className="serial-port-item"
                    data-serial-port={port.path}
                    onClick={() => edit({ port: port.path })}
                  >
                    {port.path}
                    <span className="serial-port-kind">{describeKind(port.kind)}</span>
                  </button>
                </li>
              ))}
            </ul>
            <p className="serial-ports-note">
              这份清单是系统认为存在的端口 —— 列在这里不等于打得开（点一条只是把它填进下面那一栏）。
            </p>
          </>
        )}
      </section>

      <form
        className="serial-form"
        onSubmit={(event) => {
          event.preventDefault();
          submit();
        }}
      >
        <label className="serial-field serial-field-port">
          设备路径
          <input
            className="serial-input"
            data-serial-field="port"
            value={form.port}
            placeholder="/dev/ttyUSB0"
            onChange={(event) => edit({ port: event.target.value })}
          />
        </label>
        <label className="serial-field">
          波特率
          <input
            className="serial-input"
            data-serial-field="baud"
            value={form.baud}
            inputMode="numeric"
            onChange={(event) => edit({ baud: event.target.value })}
          />
        </label>
        <label className="serial-field">
          数据位
          <input
            className="serial-input"
            data-serial-field="dataBits"
            value={form.dataBits}
            inputMode="numeric"
            onChange={(event) => edit({ dataBits: event.target.value })}
          />
        </label>
        <label className="serial-field">
          停止位
          <input
            className="serial-input"
            data-serial-field="stopBits"
            value={form.stopBits}
            inputMode="numeric"
            onChange={(event) => edit({ stopBits: event.target.value })}
          />
        </label>
        <label className="serial-field">
          校验位
          <select
            className="serial-input"
            data-serial-field="parity"
            value={form.parity}
            onChange={(event) =>
              // 选项表（`PARITIES`）就是那个联合本身，所以这一处收窄不是第二份取值域。
              edit({ parity: event.target.value as SerialParams["parity"] })
            }
          >
            {PARITIES.map((parity) => (
              <option key={parity.value} value={parity.value}>
                {parity.label}
              </option>
            ))}
          </select>
        </label>
        <label className="serial-field">
          流控
          <select
            className="serial-input"
            data-serial-field="flow"
            value={form.flow}
            onChange={(event) => edit({ flow: event.target.value as SerialParams["flow"] })}
          >
            {FLOWS.map((flow) => (
              <option key={flow.value} value={flow.value}>
                {flow.label}
              </option>
            ))}
          </select>
        </label>

        {problem && (
          <p className="serial-picker-problem" data-picker-problem role="alert">
            {problem}
          </p>
        )}

        <div className="serial-picker-actions">
          <button type="button" className="serial-picker-close" onClick={onClose}>
            取消
          </button>
          <button type="submit" className="serial-picker-open" data-serial-open>
            打开
          </button>
        </div>
      </form>

      <section className="serial-pool">
        <p className="serial-pool-title">
          库里的串口配置（{serials ? `${serials.length} 条` : "读取中…"}）—— 点一行填满上面那张表单
        </p>
        {serialsProblem && (
          <p className="serial-pool-problem" data-pool-problem role="alert">
            {serialsProblem}
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
                  onClick={() => useEntry(serial)}
                >
                  {serial.name}
                  <span className="serial-picker-target">{framing(serial)}</span>
                </button>
              </li>
            ))}
          </ul>
        )}
      </section>
    </div>
  );
}
