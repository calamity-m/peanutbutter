---
name: Docker Images
tags:
  - docker
  - images
---

# Docker Images

## List images

```bash
docker images
```

## Build an image

```bash
docker build -t <@tag> <@context:?.>
```

## Build with a specific Dockerfile

```bash
docker build -f <@dockerfile:rg . --files -g "Dockerfile*"> -t <@tag> <@context:?.>
```

## Push an image

```bash
docker push <@image:docker images --format '{{.Repository}}:{{.Tag}}'>
```

## Remove dangling images

```bash
docker image prune -f
```
