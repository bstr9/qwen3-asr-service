from abc import ABC, abstractmethod

class BaseEngine(ABC):
    """推理引擎抽象基类"""

    @abstractmethod
    def load(self):
        """加载模型到内存/显存"""
        pass

    @abstractmethod
    def transcribe(self, audio_path: str) -> str:
        """执行语音识别，返回文本"""
        pass

    def unload(self):
        """释放模型资源"""
        pass

    @property
    def is_loaded(self) -> bool:
        """模型是否已加载"""
        return False
