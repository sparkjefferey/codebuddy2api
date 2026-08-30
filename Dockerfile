FROM python:3.12-slim

WORKDIR /app

COPY requirements.txt .
RUN pip install --no-cache-dir -r requirements.txt

# 网关所有 Python 模块 + 示例配置 + 前端
COPY converter.py accounts.py config.py routing.py models_catalog.py billing.py \
     ccswitch.py \
     desensitize.py responses_adapter.py responses_projection.py anthropic_adapter.py ./
COPY config.example.json ./
COPY web/ web/

EXPOSE 8787

# 挂载宿主 auth 目录后,用 CODEBUDDY_AUTH_DIR 指定;默认跑 CN/global 自动发现
CMD ["python3", "converter.py", "--host", "0.0.0.0", "--port", "8787", "--skip-check", "--desensitize"]