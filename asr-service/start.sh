#!/usr/bin/env bash
set -e

SCRIPT_DIR="$(cd "$(dirname "$0")" && pwd)"
cd "$SCRIPT_DIR"

# 检查 venv
if [ ! -d "venv" ]; then
    echo "[WARN] 虚拟环境未创建，正在初始化..."
    bash setup.sh
fi

source venv/bin/activate
python -m app.main
