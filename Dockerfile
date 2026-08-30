FROM python:3.12-slim

WORKDIR /app

COPY requirements.txt .
RUN pip install --no-cache-dir -r requirements.txt

# Python 包 + 配置模板 + 控制台前端
COPY src/ src/
COPY templates/ templates/
COPY web/ web/

# 让 python -m codebuddy2api.converter 能解析到包
ENV PYTHONPATH=/app/src
ENV PATH=/app/.venv/bin:$PATH

EXPOSE 8787

# 挂载宿主 auth 目录后,用 CODEBUDDY_AUTH_DIR 指定;默认跑 CN/global 自动发现
CMD ["python3", "-m", "codebuddy2api.converter", "--host", "0.0.0.0", "--port", "8787", "--skip-check", "--desensitize"]