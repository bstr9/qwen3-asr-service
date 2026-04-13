#!/usr/bin/env bash
set -e

IMAGE_NAME="lancelrq/qwen3-asr-service"

# 选择构建版本：GPU / CPU
echo "请选择构建版本："
echo "  1) GPU（默认）"
echo "  2) CPU"
read -rp "请输入 [1/2]（回车默认 1）: " variant_choice
case "${variant_choice}" in
    2)   VARIANT="cpu" ;;
    *)   VARIANT="gpu" ;;
esac

# 输入版本号
SUFFIX=""
if [ "$VARIANT" = "cpu" ]; then
    SUFFIX="-cpu"
fi
read -rp "请输入版本号（回车默认 latest）: " input_ver
VER="${input_ver:-latest}"
TAG="${VER}${SUFFIX}"

# 构建
echo ""
if [ "$VARIANT" = "cpu" ]; then
    echo "Building ${IMAGE_NAME}:${TAG} (CPU) ..."
    docker build -f Dockerfile.cpu -t "${IMAGE_NAME}:${TAG}" .
else
    echo "Building ${IMAGE_NAME}:${TAG} (GPU) ..."
    docker build -t "${IMAGE_NAME}:${TAG}" .
fi

# 输出结果
echo ""
echo "Build complete: ${IMAGE_NAME}:${TAG}"
echo ""
echo "Run example:"
if [ "$VARIANT" = "cpu" ]; then
    echo "  docker run -p 8765:8765 -v /path/to/models:/app/models ${IMAGE_NAME}:${TAG}"
else
    echo "  docker run --gpus all -p 8765:8765 -v /path/to/models:/app/models ${IMAGE_NAME}:${TAG}"
fi
