from app.config import MODEL_REPO_MAP, MODEL_LOCAL_MAP, MODEL_SOURCE
from app.engines.openvino_engine import OpenVINOEngine
from app.engines.qwen_cuda_engine import QwenCudaEngine
from app.engines.align_engine import AlignEngine


def _get_repo_id(model_key: str) -> str:
    """根据当前下载源获取模型仓库 ID"""
    return MODEL_REPO_MAP[MODEL_SOURCE][model_key]


def create_engine(engine_name: str):
    """根据引擎名称创建对应引擎实例"""
    if engine_name == "openvino":
        return OpenVINOEngine(model_path=MODEL_LOCAL_MAP.get("qwen_0_6b", ""))
    elif engine_name in ("qwen_0_6b", "qwen_1_7b"):
        return QwenCudaEngine(
            model_path=MODEL_LOCAL_MAP[engine_name],
            repo_id=_get_repo_id(engine_name),
        )
    else:
        raise ValueError(f"未知引擎: {engine_name}")


def create_aligner() -> AlignEngine:
    """创建对齐引擎实例"""
    return AlignEngine(
        model_path=MODEL_LOCAL_MAP["aligner"],
        repo_id=_get_repo_id("aligner"),
    )
