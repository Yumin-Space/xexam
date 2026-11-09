# CCARP 模拟考：只用 Python 标准库，不需要 pip install。
FROM python:3.12-slim

ENV PYTHONUNBUFFERED=1 \
    PYTHONDONTWRITEBYTECODE=1 \
    CCARP_DATA=/data \
    CCARP_HOST=0.0.0.0 \
    CCARP_PORT=8765

# 不用 root 运行
RUN useradd --create-home --uid 10001 app && mkdir /data && chown app:app /data

WORKDIR /srv/app
COPY --chown=app:app server.py ./
COPY --chown=app:app app ./app
COPY --chown=app:app web ./web

USER app
VOLUME ["/data"]
EXPOSE 8765

HEALTHCHECK --interval=30s --timeout=3s --start-period=5s --retries=3 \
  CMD python -c "import urllib.request,sys; sys.exit(0 if urllib.request.urlopen('http://127.0.0.1:8765/api/info', timeout=2).status == 200 else 1)"

CMD ["python", "server.py", "--no-browser"]
