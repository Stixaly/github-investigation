FROM node:24-alpine@sha256:ebfe2f90462722a7a4de65e91990e97fe0d401c70e0e762c5b53302f905ec1c1 AS ui
WORKDIR /ui
COPY frontend/package*.json ./
RUN npm ci
COPY frontend/ ./
RUN npm run build

FROM python:3.13-alpine@sha256:2d9aefe2fef018a7eb2c13064c89c71929800fd2e5dccdbf52ea5da5bb8d929a AS runtime
ENV PYTHONDONTWRITEBYTECODE=1 PYTHONUNBUFFERED=1 PIP_DISABLE_PIP_VERSION_CHECK=1
RUN addgroup -g 10001 app && adduser -D -u 10001 -G app app
WORKDIR /app
COPY requirements/runtime.lock /app/requirements/runtime.lock
RUN pip install --no-cache-dir --require-hashes -r requirements/runtime.lock
COPY investigator/ /app/investigator/
COPY --from=ui /ui/dist /app/static
USER 10001:10001
EXPOSE 8000
CMD ["python", "-m", "uvicorn", "investigator.api:app", "--host", "0.0.0.0", "--port", "8000", "--no-proxy-headers"]

FROM runtime AS dev
USER root
COPY requirements/dev.lock /app/requirements/dev.lock
RUN pip install --no-cache-dir --require-hashes -r requirements/dev.lock
COPY tests/ /app/tests/
COPY pyproject.toml /app/pyproject.toml
USER 10001:10001
CMD ["python", "-m", "pytest", "-p", "no:cacheprovider"]
