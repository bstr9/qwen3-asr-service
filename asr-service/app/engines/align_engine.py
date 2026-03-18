import logging
from app.engines.base import BaseEngine
from app.utils.model_manager import ensure_model

logger = logging.getLogger(__name__)

class AlignEngine(BaseEngine):
    def __init__(self, model_path: str, repo_id: str):
        self._model_path = model_path
        self._repo_id = repo_id
        self._model = None

    def load(self):
        ensure_model(self._repo_id, self._model_path)
        # TODO: 加载对齐模型
        logger.info(f"对齐模型已加载: {self._model_path}")

    def align(self, audio_path: str, text: str) -> list[dict]:
        """
        对齐音频和文本，返回字级别时间戳。
        返回格式: [{"word": "你好", "start": 0.0, "end": 1.2}, ...]
        """
        if not self.is_loaded:
            self.load()
        # TODO: 实现对齐逻辑
        return []

    def transcribe(self, audio_path: str) -> str:
        raise NotImplementedError("AlignEngine 不支持 transcribe")

    def unload(self):
        self._model = None
        logger.info("对齐模型已卸载")

    @property
    def is_loaded(self) -> bool:
        return self._model is not None
