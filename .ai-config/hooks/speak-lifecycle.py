#!/usr/bin/env python3
"""生命周期 TTS（Cursor / Claude Code 等）。

- 播哪些事件：由 hooks.json 注册决定；不想播就从 manifest 删掉该事件。
- 开关与音色：改下方「配置」常量即可，无需额外 env 文件。
- 多条提示按到达顺序排队播放，不打断、不丢弃。
- 用法：./hooks/speak-lifecycle.py <EventName>
"""
from __future__ import annotations

import fcntl
import json
import os
import platform
import re
import select
import shutil
import subprocess
import sys

# --- 配置（改这里即可） ---
TTS_ENABLED = True
TTS_VOICE = "Ting-Ting"  # macOS `say -v ?`
TTS_LANG = "zh-CN"  # Linux espeak-ng

EVENT_PHRASES: dict[str, str] = {
    "sessionstart": "会话开始",
    "sessionend": "会话结束",
    "pretooluse": "准备执行工具",
    "posttooluse": "工具执行完成",
    "posttoolusefailure": "工具执行失败",
    "beforeshellexecution": "即将运行终端命令",
    "aftershellexecution": "终端命令结束",
    "beforesubmitprompt": "提交问题",
    "beforemcpexecution": "即将调用工具服务",
    "aftermcpexecution": "工具服务调用结束",
    "beforereadfile": "读取文件",
    "afterfileedit": "文件已修改",
    "stop": "本轮任务结束",
    "subagentstart": "子代理启动",
    "subagentstop": "子代理结束",
    "precompact": "上下文压缩",
    "afteragentresponse": "代理回复完成",
    "afteragentthought": "思考完成",
    "beforetabfileread": "补全读取文件",
    "aftertabfileedit": "补全编辑文件",
    "workspaceopen": "工作区已打开",
}


def normalize_event(name: str) -> str:
    return re.sub(r"[^a-z0-9]", "", name.lower())


def read_stdin_json() -> dict:
    try:
        if sys.stdin.closed or sys.stdin.isatty():
            return {}
        # 给 IDE 注入 stdin 的时间稍长一点，避免在慢启动平台（Windows / 重 IDE）丢首字节。
        ready, _, _ = select.select([sys.stdin], [], [], 0.2)
        if not ready:
            return {}
        raw_bytes = os.read(sys.stdin.fileno(), 1 << 20)
    except (OSError, ValueError):
        return {}
    raw = raw_bytes.decode("utf-8", errors="ignore")
    if not raw.strip():
        return {}
    try:
        return json.loads(raw)
    except json.JSONDecodeError:
        return {}


def resolve_event(argv: list[str], payload: dict) -> str:
    if len(argv) > 1 and argv[1].strip():
        return normalize_event(argv[1])
    if payload.get("hook_event_name"):
        return normalize_event(str(payload["hook_event_name"]))
    if payload.get("tool_name"):
        return "pretooluse"
    if payload.get("session_id") and not payload.get("tool_name"):
        return "sessionstart"
    return "unknown"


def phrase_for(event_key: str, payload: dict) -> str:
    base = EVENT_PHRASES.get(event_key, f"生命周期 {event_key}")
    return base


def speak_blocking(text: str) -> None:
    if platform.system() == "Darwin" and shutil.which("say"):
        args = ["say"]
        if TTS_VOICE:
            args.extend(["-v", TTS_VOICE])
        args.append(text)
        subprocess.run(
            args,
            stdout=subprocess.DEVNULL,
            stderr=subprocess.DEVNULL,
            check=False,
        )
        return

    if shutil.which("espeak-ng"):
        subprocess.run(
            ["espeak-ng", "-v", TTS_LANG, text],
            stdout=subprocess.DEVNULL,
            stderr=subprocess.DEVNULL,
            check=False,
        )
        return
    if shutil.which("espeak"):
        subprocess.run(
            ["espeak", text],
            stdout=subprocess.DEVNULL,
            stderr=subprocess.DEVNULL,
            check=False,
        )
        return
    if shutil.which("spd-say"):
        subprocess.run(
            ["spd-say", text],
            stdout=subprocess.DEVNULL,
            stderr=subprocess.DEVNULL,
            check=False,
        )


def speak_serialized(text: str) -> None:
    """纯内存串行：对脚本文件加锁，播放完成后释放。"""
    lock_handle = open(__file__, "r", encoding="utf-8")
    try:
        fcntl.flock(lock_handle.fileno(), fcntl.LOCK_EX)
        speak_blocking(text)
    finally:
        fcntl.flock(lock_handle.fileno(), fcntl.LOCK_UN)
        lock_handle.close()


def main() -> None:
    payload = read_stdin_json()
    event_key = resolve_event(sys.argv, payload)
    # 统一输出 JSON：所有平台（Cursor / Claude / Codex）都会按 JSON 决策；
    # `{"continue":true}` 表示"不阻断、放行"，等价于空 stdout 决策 + 不发送 permissionDecision。
    sys.stdout.write('{"continue":true}\n')
    sys.stdout.flush()

    if not TTS_ENABLED:
        return
    if event_key == "unknown":
        return

    speak_serialized(phrase_for(event_key, payload))


if __name__ == "__main__":
    main()
