import os
import shutil
import logging
from app.engines.base import BaseEngine
from app.engines.align_engine import AlignEngine
from app.pipeline.audio_splitter import split_audio
from app.config import AUDIO_CHUNKS_DIR, DEFAULT_CHUNK_SIZE, GPU_CHUNK_SIZE

logger = logging.getLogger(__name__)

class ASRPipeline:
    def __init__(self, engine: BaseEngine, aligner: AlignEngine = None, is_gpu: bool = False):
        self.engine = engine
        self.aligner = aligner
        self.chunk_size = GPU_CHUNK_SIZE if is_gpu else DEFAULT_CHUNK_SIZE

    def run(self, audio_path: str, task_id: str = None, progress_callback=None) -> dict:
        """
        执行完整 ASR 流程。

        参数:
            audio_path: 音频文件路径
            task_id: 任务 ID（用于临时文件隔离）
            progress_callback: 进度回调 fn(progress: float)

        返回:
            {"segments": [{"start": float, "end": float, "text": str, "words": list?}]}
        """
        # 1. 创建任务专属临时目录
        task_chunk_dir = os.path.join(AUDIO_CHUNKS_DIR, task_id or "default")
        os.makedirs(task_chunk_dir, exist_ok=True)

        try:
            # 2. 切片
            chunks = split_audio(audio_path, self.chunk_size, task_chunk_dir)
            total = len(chunks)
            segments = []

            # 3. 逐片段推理
            for i, chunk in enumerate(chunks):
                text = self.engine.transcribe(chunk.path)

                segment = {
                    "start": chunk.start,
                    "end": chunk.start + chunk.duration,
                    "text": text,
                }

                # 4. 可选：对齐
                if self.aligner:
                    segment["words"] = self.aligner.align(chunk.path, text)

                segments.append(segment)

                # 5. 更新进度
                if progress_callback:
                    progress_callback((i + 1) / total)

            # 6. 对齐引擎用完即卸
            if self.aligner:
                self.aligner.unload()

            return {"segments": segments}

        finally:
            # 7. 清理临时切片文件
            shutil.rmtree(task_chunk_dir, ignore_errors=True)
