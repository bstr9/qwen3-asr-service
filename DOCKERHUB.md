**[中文](#特性)** | **[English](#features)**

---

基于 Qwen3-ASR 的开箱即用长语音识别 API 服务，支持 GPU（CUDA）和 CPU（OpenVINO INT8）双模式推理。

### 特性

- 支持 1s ~ 4 小时的长语音文件，自动 VAD 切片处理
- 多格式支持：WAV / MP3 / FLAC / M4A / AAC / OGG / WMA / AMR / OPUS
- 异步任务队列，提交后轮询结果
- 句子级 / 单词级时间戳（GPU 模式）
- 可选标点恢复（CT-Transformer）
- 可选 Bearer Token API 认证（兼容 OpenAI 格式）
- 内置 Web UI，支持音频上传、进度展示、结果播放和导出

### 快速启动

#### GPU 模式

```bash
docker run -d --gpus all \
  -p 8765:8765 \
  -v /path/to/models:/app/models \
  -v /path/to/logs:/app/logs \
  --name qwen3-asr-service \
  lancelrq/qwen3-asr-service:latest
```

#### CPU 模式

```bash
docker run -d \
  -p 8765:8765 \
  -v /path/to/models:/app/models \
  -v /path/to/logs:/app/logs \
  --name qwen3-asr-service \
  lancelrq/qwen3-asr-service:latest \
  --device cpu --model-size 0.6b
```

首次启动会自动下载模型文件，挂载 `/app/models` 目录可持久化模型避免重复下载。

### Docker Compose

```yaml
services:
  asr:
    image: lancelrq/qwen3-asr-service:latest
    ports:
      - "8765:8765"
    environment:
      # - ASR_API_KEY=sk-your-key-here
    volumes:
      - ./models:/app/models
      - ./logs:/app/logs
    deploy:
      resources:
        reservations:
          devices:
            - driver: nvidia
              count: all
              capabilities: [gpu]
    command:
      - --model-size=0.6b
      - --device=auto
      - --model-source=modelscope
      - --enable-align
      - --web
    restart: unless-stopped
```

### 启动参数

所有参数均通过 `command` 传入：

| 参数 | 取值 | 默认值 | 说明 |
|------|------|--------|------|
| `--device` | `auto` / `cuda` / `cpu` | `auto` | 运行设备，auto 自动检测 |
| `--model-size` | `0.6b` / `1.7b` | 自动选择 | ASR 模型大小 |
| `--enable-align` / `--no-align` | - | 启用 | 对齐模型（单词级时间戳） |
| `--use-punc` | - | 关闭 | 标点恢复 |
| `--model-source` | `modelscope` / `huggingface` | `modelscope` | 模型下载源 |
| `--port` | 端口号 | `8765` | 监听端口 |
| `--web` | - | 关闭 | 启用 Web UI（访问 `/web-ui`） |
| `--max-segment` | 秒数 | `5` | VAD 切片合并最大时长 |
| `--api-key` | 字符串 | 无 | API 密钥，启用 Bearer Token 认证 |
| `--max-queue-size` | 数字 | `100` | 任务队列最大长度 |

> 容器内部固定监听 `0.0.0.0`，通过 `-p` 映射端口即可从外部访问。

### 数据卷

| 容器路径 | 说明 |
|---------|------|
| `/app/models` | 模型文件（首次启动自动下载，建议挂载持久化） |
| `/app/logs` | 服务日志 |

### API 使用

#### 提交 ASR 任务

```bash
curl -X POST http://localhost:8765/v1/asr \
  -F "file=@audio.wav"
```

如启用了 API 认证：

```bash
curl -X POST http://localhost:8765/v1/asr \
  -H "Authorization: Bearer sk-your-key-here" \
  -F "file=@audio.wav"
```

响应：

```json
{"task_id": "550e8400-e29b-41d4-a716-446655440000"}
```

#### 查询结果

```bash
curl http://localhost:8765/v1/asr/{task_id}
```

响应：

```json
{
  "task_id": "550e8400-...",
  "status": "completed",
  "progress": 1.0,
  "result": {
    "segments": [
      {"start": 0.0, "end": 3.2, "text": "识别的文本内容"}
    ],
    "full_text": "识别的文本内容"
  }
}
```

任务状态：`pending` → `processing` → `completed` / `failed`

#### 健康检查

```bash
curl http://localhost:8765/v1/health
```

### 运行模式对比

| | GPU 模式 | CPU 模式 |
|--|---------|---------|
| 推理引擎 | PyTorch (CUDA) | OpenVINO (INT8) |
| 对齐（字级时间戳） | 支持 | 不支持 |
| 显存/内存需求 | ~2-8GB 显存 | ~4-6GB 内存 |
| 模型来源 | ModelScope / HuggingFace | HuggingFace |

> `--device auto` 时根据显存自动选择：>=6GB 用 1.7B，4-6GB 用 0.6B，无 GPU 回退 CPU。

### 源码

[GitHub: qwen3-asr-service](https://github.com/LanceLRQ/qwen3-asr-service)

---

A ready-to-use long-form speech recognition API service based on Qwen3-ASR, supporting both GPU (CUDA) and CPU (OpenVINO INT8) inference.

### Features

- Long audio support from 1s to 4 hours with automatic VAD segmentation
- Multiple formats: WAV / MP3 / FLAC / M4A / AAC / OGG / WMA / AMR / OPUS
- Async task queue — submit and poll for results
- Sentence-level and word-level timestamps (GPU mode)
- Optional punctuation restoration (CT-Transformer)
- Optional Bearer Token API authentication (OpenAI-compatible format)
- Built-in Web UI for uploading audio, tracking progress, playing results, and exporting

### Quick Start

#### GPU Mode

```bash
docker run -d --gpus all \
  -p 8765:8765 \
  -v /path/to/models:/app/models \
  -v /path/to/logs:/app/logs \
  --name qwen3-asr-service \
  lancelrq/qwen3-asr-service:latest
```

#### CPU Mode

```bash
docker run -d \
  -p 8765:8765 \
  -v /path/to/models:/app/models \
  -v /path/to/logs:/app/logs \
  --name qwen3-asr-service \
  lancelrq/qwen3-asr-service:latest \
  --device cpu --model-size 0.6b
```

Models are downloaded automatically on first startup. Mount `/app/models` to persist them across restarts.

### Docker Compose

```yaml
services:
  asr:
    image: lancelrq/qwen3-asr-service:latest
    ports:
      - "8765:8765"
    environment:
      # - ASR_API_KEY=sk-your-key-here
    volumes:
      - ./models:/app/models
      - ./logs:/app/logs
    deploy:
      resources:
        reservations:
          devices:
            - driver: nvidia
              count: all
              capabilities: [gpu]
    command:
      - --model-size=0.6b
      - --device=auto
      - --model-source=modelscope
      - --enable-align
      - --web
    restart: unless-stopped
```

### Parameters

All parameters are passed via `command`:

| Parameter | Values | Default | Description |
|-----------|--------|---------|-------------|
| `--device` | `auto` / `cuda` / `cpu` | `auto` | Device selection, auto-detects GPU |
| `--model-size` | `0.6b` / `1.7b` | Auto | ASR model size |
| `--enable-align` / `--no-align` | - | Enabled | Forced alignment (word-level timestamps) |
| `--use-punc` | - | Disabled | Punctuation restoration |
| `--model-source` | `modelscope` / `huggingface` | `modelscope` | Model download source |
| `--port` | Port number | `8765` | Listening port |
| `--web` | - | Disabled | Enable Web UI (access `/web-ui`) |
| `--max-segment` | Seconds | `5` | Max VAD segment merge duration |
| `--api-key` | String | None | API key, enables Bearer Token authentication |
| `--max-queue-size` | Number | `100` | Max task queue size |

> The container always listens on `0.0.0.0` internally. Use `-p` to map the port for external access.

### Volumes

| Container Path | Description |
|---------------|-------------|
| `/app/models` | Model files (auto-downloaded on first run, mount to persist) |
| `/app/logs` | Service logs |

### API Usage

#### Submit ASR Task

```bash
curl -X POST http://localhost:8765/v1/asr \
  -F "file=@audio.wav"
```

With API authentication enabled:

```bash
curl -X POST http://localhost:8765/v1/asr \
  -H "Authorization: Bearer sk-your-key-here" \
  -F "file=@audio.wav"
```

Response:

```json
{"task_id": "550e8400-e29b-41d4-a716-446655440000"}
```

#### Query Result

```bash
curl http://localhost:8765/v1/asr/{task_id}
```

Response:

```json
{
  "task_id": "550e8400-...",
  "status": "completed",
  "progress": 1.0,
  "result": {
    "segments": [
      {"start": 0.0, "end": 3.2, "text": "Transcribed text content"}
    ],
    "full_text": "Transcribed text content"
  }
}
```

Task status flow: `pending` → `processing` → `completed` / `failed`

#### Health Check

```bash
curl http://localhost:8765/v1/health
```

### Mode Comparison

| | GPU Mode | CPU Mode |
|--|---------|---------|
| Inference Engine | PyTorch (CUDA) | OpenVINO (INT8) |
| Alignment (word timestamps) | Supported | Not supported |
| VRAM / Memory | ~2-8GB VRAM | ~4-6GB RAM |
| Model Source | ModelScope / HuggingFace | HuggingFace |

> With `--device auto`, the service selects automatically: >=6GB VRAM uses 1.7B, 4-6GB uses 0.6B, no GPU falls back to CPU.

### Source Code

[GitHub: qwen3-asr-service](https://github.com/LanceLRQ/qwen3-asr-service)
