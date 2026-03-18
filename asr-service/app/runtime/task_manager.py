import queue
import threading
import uuid
import logging
import time
from concurrent.futures import ThreadPoolExecutor, TimeoutError as FuturesTimeoutError
from datetime import datetime
from app.config import TASK_TIMEOUT, TASK_RESULT_TTL, TASK_CLEANUP_INTERVAL

logger = logging.getLogger(__name__)


class TaskManager:
    def __init__(self, max_queue_size=100):
        self._queue = queue.Queue(maxsize=max_queue_size)
        self._tasks = {}  # task_id -> task_dict
        self._lock = threading.Lock()
        self._worker_thread = None
        self._cleanup_thread = None
        self._process_fn = None
        self._executor = ThreadPoolExecutor(max_workers=1)

    def set_processor(self, fn):
        """注入任务处理函数: fn(task_dict) -> result"""
        self._process_fn = fn

    def start(self):
        """启动工作线程和清理线程"""
        self._worker_thread = threading.Thread(target=self._worker, daemon=True)
        self._worker_thread.start()
        self._cleanup_thread = threading.Thread(target=self._cleanup_loop, daemon=True)
        self._cleanup_thread.start()
        logger.info("任务工作线程和清理线程已启动")

    def submit(self, file_path: str, language: str | None = None) -> str:
        """提交任务，返回 task_id"""
        task_id = str(uuid.uuid4())
        task = {
            "task_id": task_id,
            "status": "pending",
            "progress": 0.0,
            "file_path": file_path,
            "language": language,
            "result": None,
            "error": None,
            "created_at": datetime.now().isoformat(),
            "finished_at": None,
        }

        with self._lock:
            self._tasks[task_id] = task

        self._queue.put_nowait(task_id)  # 队列满时抛出 queue.Full
        logger.info(f"任务已提交: {task_id}")
        return task_id

    def get_task(self, task_id: str) -> dict | None:
        """查询任务状态"""
        with self._lock:
            return self._tasks.get(task_id)

    def update_progress(self, task_id: str, progress: float):
        """更新任务进度"""
        with self._lock:
            if task_id in self._tasks:
                self._tasks[task_id]["progress"] = progress

    def _worker(self):
        """工作线程：串行处理任务，使用线程池实现真超时"""
        while True:
            task_id = self._queue.get()

            with self._lock:
                task = self._tasks.get(task_id)
                if not task:
                    continue
                task["status"] = "processing"

            start_time = time.time()
            try:
                future = self._executor.submit(self._process_fn, task)
                result = future.result(timeout=TASK_TIMEOUT)
                elapsed = time.time() - start_time

                with self._lock:
                    task["status"] = "completed"
                    task["progress"] = 1.0
                    task["result"] = result
                    task["finished_at"] = time.time()
                logger.info(f"任务完成: {task_id} ({elapsed:.1f}s)")
            except FuturesTimeoutError:
                elapsed = time.time() - start_time
                future.cancel()
                with self._lock:
                    task["status"] = "failed"
                    task["error"] = f"处理超时（>{TASK_TIMEOUT}s）"
                    task["finished_at"] = time.time()
                logger.error(f"任务超时: {task_id} ({elapsed:.0f}s)")
            except Exception as e:
                with self._lock:
                    task["status"] = "failed"
                    task["error"] = str(e)
                    task["finished_at"] = time.time()
                logger.error(f"任务失败: {task_id}, 错误: {e}")
            finally:
                self._queue.task_done()

    def _cleanup_loop(self):
        """定期清理已完成/失败的过期任务"""
        while True:
            time.sleep(TASK_CLEANUP_INTERVAL)
            self._cleanup_expired_tasks()

    def _cleanup_expired_tasks(self):
        """清理超过 TTL 的已终结任务"""
        now = time.time()
        expired = []

        with self._lock:
            for task_id, task in self._tasks.items():
                if task["status"] in ("completed", "failed") and task.get("finished_at"):
                    if now - task["finished_at"] > TASK_RESULT_TTL:
                        expired.append(task_id)
            for task_id in expired:
                del self._tasks[task_id]

        if expired:
            logger.info(f"已清理 {len(expired)} 个过期任务")
