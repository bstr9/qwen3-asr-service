import os
import soundfile as sf
import logging

logger = logging.getLogger(__name__)

class AudioChunk:
    """音频片段"""
    def __init__(self, path: str, start: float, duration: float):
        self.path = path
        self.start = start
        self.duration = duration

def split_audio(audio_path: str, chunk_size: int = 30, output_dir: str = None) -> list[AudioChunk]:
    """
    将音频文件按 chunk_size（秒）切分。

    参数:
        audio_path: 源音频文件路径
        chunk_size: 每段时长（秒）
        output_dir: 切片输出目录

    返回:
        AudioChunk 列表
    """
    data, samplerate = sf.read(audio_path)
    total_duration = len(data) / samplerate
    samples_per_chunk = chunk_size * samplerate

    chunks = []
    offset = 0
    index = 0

    while offset < len(data):
        end = min(offset + samples_per_chunk, len(data))
        chunk_data = data[offset:end]
        chunk_duration = len(chunk_data) / samplerate
        chunk_start = offset / samplerate

        # 写入临时文件
        chunk_filename = f"chunk_{index:04d}.wav"
        chunk_path = os.path.join(output_dir, chunk_filename) if output_dir else chunk_filename
        sf.write(chunk_path, chunk_data, samplerate)

        chunks.append(AudioChunk(
            path=chunk_path,
            start=chunk_start,
            duration=chunk_duration,
        ))

        offset = end
        index += 1

    logger.info(f"音频切片完成: {audio_path} -> {len(chunks)} 片段 (每段 {chunk_size}s)")
    return chunks
