#!/usr/bin/env bash
set -euo pipefail

DEFAULT_ROOT="${HOME}/.local/share/visualner/models"
if [[ -n "${GLINER25_MODEL_DIR:-}" ]]; then
  DEST="${GLINER25_MODEL_DIR}"
else
  MODEL_ROOT="${VISUALNER_MODEL_DIR:-${DEFAULT_ROOT}}"
  DEST="${MODEL_ROOT}/gliner2.5-multi-v1-onnx"
fi

if ! command -v hf >/dev/null 2>&1; then
  echo "install_gliner25_model: install Hugging Face CLI first (python3 -m pip install huggingface_hub)" >&2
  exit 1
fi

mkdir -p "${DEST}"
hf download jugaadsrl/gliner2.5-multi-v1-onnx \
  --revision 3de2c11f55853b7919f985a59bfc8df27b5ec991 \
  --include 'fp16_25/*' \
  --local-dir "${DEST}"

RUNTIME_DIR="${DEST}/.onnxruntime"
python3 -m pip install --target "${RUNTIME_DIR}" --no-deps 'onnxruntime==1.23.2'
RUNTIME_LIB="${RUNTIME_DIR}/onnxruntime/capi/libonnxruntime.so.1.23.2"
if [[ ! -f "${RUNTIME_LIB}" ]]; then
  echo "install_gliner25_model: ONNX Runtime shared library was not installed at ${RUNTIME_LIB}" >&2
  exit 1
fi
install -m 0755 "${RUNTIME_LIB}" "${DEST}/libonnxruntime.so.1.23.2"
echo "GLiNER2.5 installed at ${DEST}"
