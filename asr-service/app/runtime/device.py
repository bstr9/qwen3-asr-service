import torch
import logging

logger = logging.getLogger(__name__)

def detect_device() -> str:
    """检测运行设备，返回设备级别标识"""
    if torch.cuda.is_available():
        vram = torch.cuda.get_device_properties(0).total_memory / 1024**3
        device_name = torch.cuda.get_device_name(0)
        logger.info(f"检测到 GPU: {device_name}, VRAM: {vram:.1f}GB")

        if vram >= 6:
            return "cuda_high"
        elif vram >= 4:
            return "cuda"

    logger.info("使用 CPU 模式")
    return "cpu"
