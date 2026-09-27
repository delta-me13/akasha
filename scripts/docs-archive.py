#!/usr/bin/env python3
"""把"已完成 / 已过时"的内容从 `ROADMAP.md` 与 `docs/STATUS.md` 移入 `docs/archive/`。

规则本体在 `AGENTS.md` §8.3；入口是配方 `just docs-archive`（`AGENTS.md` §11：
脚本只经配方调用）。本文件只实现那条规则里**能机械判定**的部分 —— 判不了的留给人工：

| 来源 | 搬移条件（机械） | 去处 |
|---|---|---|
| `ROADMAP.md` | 条目以 `- [x]` 开头，连同它的验收行与 plan 指针 | `docs/archive/roadmap-completed.md`，按阶段分节 |
| `docs/STATUS.md` 的「摘要」 | 除**第一段**（最近一轮）之外的所有段落 | `docs/archive/status-history.md` 的「摘要（历史）」 |
| `docs/STATUS.md` 的「已验证为通过」 | 表格里以 `| ↑` 开头的行（被取代的读数） | 同上的「被取代的读数」 |
| `docs/STATUS.md` 的「已知问题与教训」 | 条目里出现 `已修` / `已放弃` / `已关闭` | 同上的「已处置的问题」 |

**只移动，不改写**：每一段都按原文搬过去 —— 归档同时是"这条结论当时是怎么得出的"的唯一去处，
改写它等于毁掉证据。`问题 #N` 的编号永不复用，条目移走之后编号仍然有效。

幂等：搬过一次之后源文件里不再有符合条件的行，再执行就是空操作。
"""

from __future__ import annotations

import datetime
import pathlib
import re
import sys

ROOT = pathlib.Path(__file__).resolve().parent.parent
ROADMAP = ROOT / "ROADMAP.md"
STATUS = ROOT / "docs" / "STATUS.md"
ARCHIVE = ROOT / "docs" / "archive"
ROADMAP_ARCHIVE = ARCHIVE / "roadmap-completed.md"
STATUS_ARCHIVE = ARCHIVE / "status-history.md"

#: 「已知问题与教训」里把这些词当作"已处置"的标记。它们本来就是仓库在条目里写的状态词。
RESOLVED = re.compile(r"已修|已放弃|已关闭")

#: ROADMAP 里已完成条目的行首。
DONE_ITEM = re.compile(r"^- \[x\] ")

#: 归档文件里的小节标题（追加的目标位置）。
SUMMARY_SECTION = "## 摘要（历史）"
READING_SECTION = "## 被取代的读数"
ISSUE_SECTION = "## 已处置的问题"

ROADMAP_POINTER = "> 已完成的条目（含验收行与 plan 指针）见 [归档](../archive/roadmap-completed.md)。"


def read(path: pathlib.Path) -> list[str]:
    return path.read_text(encoding="utf-8").splitlines()


def write(path: pathlib.Path, lines: list[str]) -> None:
    path.write_text("\n".join(lines) + "\n", encoding="utf-8")


def append_into_section(path: pathlib.Path, preamble: str, section: str, moved: list[str]) -> None:
    """把 `moved` 追加到 `path` 的 `section` 小节末尾；小节不存在就建在末尾。"""
    if not moved:
        return
    lines = read(path) if path.exists() else []
    if not lines:
        lines = preamble.splitlines()
    head, sections = split_h2(lines)
    while head and head[-1].strip() == "":
        head.pop()
    for index, (title, body) in enumerate(sections):
        if title == section:
            sections[index] = (title, body.rstrip("\n").splitlines() + moved)
            break
    else:
        sections.append((section, moved))
    out = list(head)
    for title, body in sections:
        out.append("")
        out.append(title)
        out.extend(body)
    write(path, collapse_blanks(out))


def split_h2(lines: list[str]) -> tuple[list[str], list[tuple[str, list[str]]]]:
    """按 `## ` 切成 (前导部分, [(标题, 正文)]）；正文里保留原文。"""
    head: list[str] = []
    sections: list[tuple[str, list[str]]] = []
    for line in lines:
        if line.startswith("## "):
            sections.append((line, []))
        elif sections:
            sections[-1][1].append(line)
        else:
            head.append(line)
    return head, sections


def collapse_blanks(lines: list[str]) -> list[str]:
    """把连续空行折成一个 —— 移走条目之后留下的空档不属于内容。"""
    out: list[str] = []
    for line in lines:
        if line == "" and out and out[-1] == "":
            continue
        out.append(line)
    return out


def archive_roadmap() -> int:
    lines = read(ROADMAP)
    out: list[str] = []
    moved: dict[str, list[str]] = {}
    stage: str | None = None
    index = 0
    while index < len(lines):
        line = lines[index]
        if line.startswith("## "):
            stage = line
            out.append(line)
            index += 1
            continue
        if DONE_ITEM.match(line) and stage is not None:
            item = [line]
            index += 1
            while index < len(lines) and lines[index].startswith((" ", "\t")):
                item.append(lines[index])
                index += 1
            moved.setdefault(stage, []).extend(item)
            continue
        out.append(line)
        index += 1

    # 每个失去条目的阶段留一行指针：插在该阶段**下一个未完成条目之前**（没有条目就放在块首）。
    for stage_title in moved:
        if _stage_has_pointer(out, stage_title):
            continue
        position = _first_item_of(out, stage_title)
        out.insert(position, ROADMAP_POINTER)

    if moved:
        preamble = (
            "# 已完成的 ROADMAP 条目（归档）\n\n"
            "> 由 `just docs-archive` 从 `ROADMAP.md` 移来（规则见 `AGENTS.md` §8.3），"
            "**只移动、不改写**。\n"
            "> 未完成的条目仍在 [`ROADMAP.md`](../../ROADMAP.md)；"
            "plan 索引见 [`../plans/README.md`](../plans/README.md)。"
        )
        for stage_title, items in moved.items():
            append_into_section(ROADMAP_ARCHIVE, preamble, stage_title, items)
        write(ROADMAP, collapse_blanks(out))
    return sum(len(items) for items in moved.values())


def _stage_bounds(lines: list[str], stage_title: str) -> tuple[int, int]:
    """某个阶段在 `lines` 里的 [起, 止) 行号；找不到就是空区间。"""
    start = next((i for i, line in enumerate(lines) if line == stage_title), None)
    if start is None:
        return (0, 0)
    end = next((i for i, line in enumerate(lines[start + 1 :], start + 1) if line.startswith("## ")), len(lines))
    return (start, end)


def _stage_has_pointer(lines: list[str], stage_title: str) -> bool:
    start, end = _stage_bounds(lines, stage_title)
    return any(line == ROADMAP_POINTER for line in lines[start:end])


def _first_item_of(lines: list[str], stage_title: str) -> int:
    """该阶段里第一个 `- [` 条目的行号；没有条目就返回块尾。"""
    start, end = _stage_bounds(lines, stage_title)
    for position in range(start, end):
        if lines[position].startswith("- ["):
            return position
    return end


def archive_status(today: str) -> dict[str, int]:
    lines = read(STATUS)
    head, sections = split_h2(lines)
    counts = {"摘要": 0, "读数": 0, "问题": 0}
    moved_summary: list[str] = []
    moved_readings: list[str] = []
    moved_issues: list[str] = []
    rebuilt: list[tuple[str, list[str]]] = []

    for title, body in sections:
        if title.startswith("## 摘要"):
            blocks: list[list[str]] = [[]]
            for line in body:
                if line == "":
                    blocks.append([])
                else:
                    blocks[-1].append(line)
            blocks = [block for block in blocks if block]
            if not blocks:
                rebuilt.append((title, body))
            else:
                first, *rest = blocks
                for block in rest:
                    moved_summary.extend(["", *block])
                    counts["摘要"] += len(block)
                rebuilt.append((title, ["", *first]))
        elif title.startswith("## 已验证为通过"):
            kept_body = [line for line in body if not line.startswith("| ↑")]
            moved_readings.extend([line for line in body if line.startswith("| ↑")])
            counts["读数"] = len(body) - len(kept_body)
            rebuilt.append((title, kept_body))
        elif title.startswith("## 已知问题与教训"):
            kept_body, taken = _split_issues(body)
            moved_issues.extend(taken)
            counts["问题"] = len(taken)
            rebuilt.append((title, kept_body))
        else:
            rebuilt.append((title, body))

    if any(counts.values()):
        preamble = (
            "# STATUS 的历史（归档）\n\n"
            "> 由 `just docs-archive` 从 [`STATUS.md`](../STATUS.md) 移来（规则见 `AGENTS.md` §8.3），"
            "**只移动、不改写**。\n"
            "> `问题 #N` 的编号永不复用 —— 条目移走之后编号仍然有效，原文就在这里。\n\n"
            f"最近一次归档：{today}。"
        )
        append_into_section(STATUS_ARCHIVE, preamble, SUMMARY_SECTION, moved_summary)
        append_into_section(STATUS_ARCHIVE, preamble, READING_SECTION, moved_readings)
        append_into_section(STATUS_ARCHIVE, preamble, ISSUE_SECTION, moved_issues)
        out = list(head)
        for title, body in rebuilt:
            out.append("")
            out.append(title)
            out.extend(body)
        write(STATUS, collapse_blanks(out))
    return counts


def _split_issues(body: list[str]) -> tuple[list[str], list[str]]:
    """按 `^ *N. ` 切条目；整条里出现处置标记的归入"移走"，其余留在原处。"""
    kept: list[str] = []
    taken: list[str] = []
    current: list[str] | None = None
    for line in body:
        if re.match(r"^ *\d+\. ", line):
            if current is not None:
                _dispatch(current, kept, taken)
            current = [line]
        elif current is None:
            kept.append(line)
        else:
            current.append(line)
    if current is not None:
        _dispatch(current, kept, taken)
    return kept, taken


def _dispatch(entry: list[str], kept: list[str], taken: list[str]) -> None:
    (taken if RESOLVED.search("\n".join(entry)) else kept).extend(entry)


def main() -> int:
    today = datetime.date.today().isoformat()
    ARCHIVE.mkdir(parents=True, exist_ok=True)
    roadmap_items = archive_roadmap()
    counts = archive_status(today)
    print(f"ROADMAP: 移出 {roadmap_items} 行")
    print(f"STATUS: 摘要 {counts['摘要']} 行 / 被取代的读数 {counts['读数']} 行 / 已处置的问题 {counts['问题']} 行")
    if not roadmap_items and not any(counts.values()):
        print("没有可移动的内容（幂等空操作）")
    return 0


if __name__ == "__main__":
    sys.exit(main())
