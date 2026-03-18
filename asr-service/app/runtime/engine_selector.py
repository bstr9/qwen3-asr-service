import logging
from app.runtime.device import detect_device

logger = logging.getLogger(__name__)

def select_engine(mode: str = "auto") -> str:
    """
    根据设备情况选择引擎。
    mode: "auto" | "cpu" | "gpu"
    返回引擎标识: "openvino" | "qwen_0_6b" | "qwen_1_7b"
    """
    if mode == "cpu":
        return "openvino"

    device = detect_device()

    if mode == "gpu" and device == "cpu":
        logger.warning("请求 GPU 模式但未检测到可用 GPU，回退到 CPU")
        return "openvino"

    engine_map = {
        "cpu": "openvino",
        "cuda": "qwen_0_6b",
        "cuda_high": "qwen_1_7b",
    }

    engine = engine_map[device]
    logger.info(f"设备: {device} -> 引擎: {engine}")
    return engine
