# ASR Service

基于 FastAPI 的语音识别服务，支持 CPU (OpenVINO) 和 GPU (Qwen3-ASR) 两种推理模式，自动检测硬件环境并选择最优引擎。

## 快速开始

### 1. 初始化环境

```bash
cd asr-service
bash setup.sh
```

脚本会自动完成：
- 创建 Python 虚拟环境 (`venv/`)
- 根据是否有 NVIDIA GPU 安装对应版本的 PyTorch
- 安装所有项目依赖
- 创建必要的运行时目录

### 2. 启动服务

```bash
bash start.sh
```

服务默认监听 `http://127.0.0.1:8765`。

### 3. 验证服务

```bash
curl http://127.0.0.1:8765/health
```

正常响应：

```json
{"status": "ok", "device": "cpu", "engine": "openvino"}
```

## API 接口

### 提交 ASR 任务

```bash
curl -X POST http://127.0.0.1:8765/asr \
  -F "file=@/path/to/audio.wav" \
  -F "mode=auto" \
  -F "align=false"
```

| 参数 | 类型 | 默认值 | 说明 |
|------|------|--------|------|
| file | 文件 | 必填 | 音频文件 (wav/mp3 等) |
| mode | string | auto | `auto` 自动选择 / `cpu` 强制CPU / `gpu` 强制GPU |
| align | bool | false | 是否返回字级别时间戳 |

响应：

```json
{"task_id": "550e8400-e29b-41d4-a716-446655440000"}
```

### 查询任务状态

```bash
curl http://127.0.0.1:8765/asr/{task_id}
```

响应（处理中）：

```json
{"task_id": "550e8400-...", "status": "processing", "progress": 0.45, "result": null, "error": null}
```

响应（完成）：

```json
{
  "task_id": "550e8400-...",
  "status": "done",
  "progress": 1.0,
  "result": {
    "segments": [
      {"start": 0, "end": 12.5, "text": "你好大家好"}
    ]
  },
  "error": null
}
```

任务状态流转：`pending` → `processing` → `done` / `error`

### 健康检查

```bash
curl http://127.0.0.1:8765/health
```

## 项目结构

```
asr-service/
├── app/
│   ├── main.py              # 服务入口
│   ├── config.py            # 全局配置
│   ├── runtime/             # 设备检测、引擎选择、任务队列
│   ├── engines/             # 推理引擎 (OpenVINO / Qwen CUDA)
│   ├── pipeline/            # 音频切片 + ASR 流水线
│   ├── api/                 # FastAPI 路由和数据模型
│   └── utils/               # 日志、模型下载管理
├── models/                  # 模型存放 (自动下载，不提交Git)
├── cache/                   # 运行时缓存
├── logs/                    # 日志文件
├── setup.sh                 # 环境初始化
├── start.sh                 # 服务启动
└── requirements.txt         # 依赖清单
```

## 硬件适配

| 环境 | 自动选择引擎 | 模型 |
|------|-------------|------|
| 无 GPU | OpenVINO | IR 格式 (CPU优化) |
| GPU VRAM 4-6GB | Qwen CUDA | Qwen3-ASR-0.6B |
| GPU VRAM >= 6GB | Qwen CUDA | Qwen3-ASR-1.7B |

## GPU 支持（可选）

如需 GPU 加速，确保已安装 NVIDIA 驱动和 CUDA 12.1+：

```bash
nvidia-smi        # 确认驱动
nvcc --version    # 确认 CUDA
```

## 配置项

主要配置在 `app/config.py`，可按需修改：

| 配置 | 默认值 | 说明 |
|------|--------|------|
| HOST | 127.0.0.1 | 监听地址 |
| PORT | 8765 | 监听端口 |
| DEFAULT_CHUNK_SIZE | 30s | CPU 模式音频切片时长 |
| GPU_CHUNK_SIZE | 60s | GPU 模式音频切片时长 |
| MAX_QUEUE_SIZE | 100 | 最大任务队列长度 |
