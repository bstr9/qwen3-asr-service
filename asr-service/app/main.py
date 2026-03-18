import logging
import uvicorn
from fastapi import FastAPI

from app.utils.logger import setup_logger
from app.config import HOST, PORT
from app.runtime.device import detect_device
from app.runtime.engine_selector import select_engine
from app.runtime.task_manager import TaskManager
from app.engines import create_engine
from app.engines.align_engine import AlignEngine
from app.pipeline.asr_pipeline import ASRPipeline
from app.api.routes import router, init_routes

logger = logging.getLogger(__name__)

def create_app() -> FastAPI:
    """创建并配置 FastAPI 应用"""

    # 1. 配置日志
    setup_logger()
    logger.info("ASR Service 启动中...")

    # 2. 检测设备
    device = detect_device()
    logger.info(f"当前设备: {device}")

    # 3. 选择引擎
    engine_name = select_engine()
    logger.info(f"选择引擎: {engine_name}")

    # 4. 创建引擎实例并预加载模型
    engine = create_engine(engine_name)
    engine.load()
    is_gpu = device.startswith("cuda")

    # 5. 创建 Pipeline
    pipeline = ASRPipeline(engine=engine, is_gpu=is_gpu)

    # 6. 创建任务管理器
    task_manager = TaskManager()

    def process_task(task: dict):
        """任务处理函数"""
        def on_progress(p):
            task_manager.update_progress(task["task_id"], p)

        return pipeline.run(
            audio_path=task["file_path"],
            task_id=task["task_id"],
            progress_callback=on_progress,
        )

    task_manager.set_processor(process_task)
    task_manager.start()

    # 7. 创建 FastAPI 应用
    app = FastAPI(title="ASR Service", version="1.0.0")
    init_routes(task_manager, device, engine_name)
    app.include_router(router)

    logger.info(f"ASR Service 就绪，监听 {HOST}:{PORT}")
    return app


app = create_app()

if __name__ == "__main__":
    uvicorn.run("app.main:app", host=HOST, port=PORT, reload=False)
