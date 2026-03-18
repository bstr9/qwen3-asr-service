from pydantic import BaseModel

class ASRRequest(BaseModel):
    mode: str = "auto"    # auto | cpu | gpu
    align: bool = False

class ASRResponse(BaseModel):
    task_id: str

class TaskStatusResponse(BaseModel):
    task_id: str
    status: str           # pending | processing | done | error
    progress: float
    result: dict | None = None
    error: str | None = None

class HealthResponse(BaseModel):
    status: str
    device: str
    engine: str
