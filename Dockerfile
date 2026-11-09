# Xexam：只用 Python 标准库，不需要 pip install。
FROM python:3.12-slim

ENV PYTHONUNBUFFERED=1 \
    PYTHONDONTWRITEBYTECODE=1 \
    XEXAM_DATA=/data \
    XEXAM_HOST=0.0.0.0 \
    XEXAM_PORT=8765

# 不用 root 运行
RUN useradd --create-home --uid 10001 app && mkdir /data && chown app:app /data

WORKDIR /srv/app
COPY --chown=app:app server.py VERSION ./
COPY --chown=app:app app ./app
COPY --chown=app:app web ./web

USER app
VOLUME ["/data"]
EXPOSE 8765

# 不走代理（Docker 有时会往容器里注入 HTTP_PROXY），端口跟着 XEXAM_PORT 走
HEALTHCHECK --interval=30s --timeout=8s --start-period=15s --retries=3 \
  CMD python -c "import os,sys,urllib.request as u; o=u.build_opener(u.ProxyHandler({})); sys.exit(0 if o.open('http://127.0.0.1:%s/api/info' % os.environ.get('XEXAM_PORT','8765'), timeout=6).status == 200 else 1)"

CMD ["python", "server.py", "--no-browser"]
