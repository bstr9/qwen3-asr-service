import os

# 项目根目录
BASE_DIR = os.path.dirname(os.path.dirname(os.path.abspath(__file__)))

# 服务配置
HOST = "0.0.0.0"
PORT = 8765

# 模型路径
MODELS_DIR = os.path.join(BASE_DIR, "models")
ASR_MODEL_DIR = os.path.join(MODELS_DIR, "asr")
ALIGN_MODEL_DIR = os.path.join(MODELS_DIR, "align")
VAD_MODEL_DIR = os.path.join(MODELS_DIR, "vad")
PUNC_MODEL_DIR = os.path.join(MODELS_DIR, "punc")

# 模型下载源: "huggingface" | "modelscope" | "manual"
MODEL_SOURCE = "modelscope"

# 模型仓库 ID
# VAD 和标点模型仅 ModelScope 提供，HuggingFace 下也强制从 ModelScope 下载
MODEL_REPO_MAP = {
    "huggingface": {
        "qwen_0_6b": "Qwen/Qwen3-ASR-0.6B",
        "qwen_1_7b": "Qwen/Qwen3-ASR-1.7B",
        "aligner": "Qwen/Qwen3-ForcedAligner-0.6B",
    },
    "modelscope": {
        "qwen_0_6b": "Qwen/Qwen3-ASR-0.6B",
        "qwen_1_7b": "Qwen/Qwen3-ASR-1.7B",
        "aligner": "Qwen/Qwen3-ForcedAligner-0.6B",
    },
}

# 仅 ModelScope 提供的模型（不受 MODEL_SOURCE 影响，始终从 ModelScope 下载）
MODELSCOPE_ONLY_REPO_MAP = {
    "vad": "iic/speech_fsmn_vad_zh-cn-16k-common-pytorch",
    "punc": "iic/punc_ct-transformer_zh-cn-common-vocab272727-pytorch",
}

# 模型本地子目录
MODEL_LOCAL_MAP = {
    "qwen_0_6b": os.path.join(ASR_MODEL_DIR, "0.6b"),
    "qwen_1_7b": os.path.join(ASR_MODEL_DIR, "1.7b"),
    "aligner": os.path.join(ALIGN_MODEL_DIR, "0.6b"),
    "vad": os.path.join(VAD_MODEL_DIR, "fsmn"),
    "punc": os.path.join(PUNC_MODEL_DIR, "ct-transformer"),
}

# 缓存路径
CACHE_DIR = os.path.join(BASE_DIR, "cache")
AUDIO_CHUNKS_DIR = os.path.join(CACHE_DIR, "audio_chunks")
RESULTS_DIR = os.path.join(CACHE_DIR, "results")

# 日志
LOG_DIR = os.path.join(BASE_DIR, "logs")
LOG_FILE = os.path.join(LOG_DIR, "asr.log")

# 音频处理
DEFAULT_CHUNK_SIZE = 30  # 秒
GPU_CHUNK_SIZE = 60      # GPU 可用更大 chunk

# 任务队列
MAX_QUEUE_SIZE = 100
