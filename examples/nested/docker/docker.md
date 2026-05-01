---
name: Docker
tags:
  - docker
  - containers
---

# Docker

## Run a container

```bash
docker run -it --rm \
  --name <@name> \
  -p <@host_port:?8080>:<@container_port:?80> \
  <@image:docker images --format '{{.Repository}}:{{.Tag}}'>
```

## Execute a shell in a running container

Requires `fzf` for interactive container selection.

```bash
(
  sel=$(
    docker ps --format '{{.Names}}\t{{.Image}}\t{{.Status}}' | \
    column -t -s $'\t' | \
    fzf --ansi --header='name | image | status' --prompt='container > '
  ) || exit
  c=$(awk '{print $1}' <<< "$sel")
  docker exec -it "$c" bash 2>/dev/null || docker exec -it "$c" sh
)
```

## View container logs

```bash
docker logs -f <@container:docker ps --format '{{.Names}}'>
```

## Remove all stopped containers

```bash
docker container prune -f
```
