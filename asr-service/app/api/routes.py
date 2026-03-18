import os
import uuid
import logging
from fastapi import APIRouter, UploadFile, File, Form
from app.api.schemas import ASRResponse, TaskStatusResponse, HealthResponse
from app.config import CACHE_DIR

logger = logging.getLogger(__name__)
router = APIRouter()

# 这些变量由 main.py 启动时注入
task_manager = None
current_device = None
current_engine_name = None


def init_routes(_task_manager, _device, _engine_name):
    """注入运行时依赖"""
    global task_manager, current_device, current_engine_name
    task_manager = _task_manager
    current_device = _device
    current_engine_name = _engine_name


@router.post("/asr", response_model=ASRResponse)
async def submit_asr(
    file: UploadFile = File(...),
    mode: str = Form("auto"),
    align: bool = Form(False),
):
    """提交 ASR 任务"""
    # 1. 保存上传文件
    upload_dir = os.path.join(CACHE_DIR, "uploads")
    os.makedirs(upload_dir, exist_ok=True)

    file_id = str(uuid.uuid4())
    file_ext = os.path.splitext(file.filename)[1] or ".wav"
    save_path = os.path.join(upload_dir, f"{file_id}{file_ext}")

    with open(save_path, "wb") as f:
        content = await file.read()
        f.write(content)

    # 2. 提交到任务队列
    task_id = task_manager.submit(
        file_path=save_path,
        mode=mode,
        align=align,
    )

    return ASRResponse(task_id=task_id)


@router.get("/asr/{task_id}", response_model=TaskStatusResponse)
async def get_task_status(task_id: str):
    """查询任务状态"""
    task = task_manager.get_task(task_id)
    if not task:
        return TaskStatusResponse(
            task_id=task_id,
            status="not_found",
            progress=0.0,
        )

    return TaskStatusResponse(
        task_id=task["task_id"],
        status=task["status"],
        progress=task["progress"],
        result=task.get("result"),
        error=task.get("error"),
    )


@router.get("/health", response_model=HealthResponse)
async def health_check():
    """健康检查"""
    return HealthResponse(
        status="ok",
        device=current_device or "unknown",
        engine=current_engine_name or "unknown",
    )
