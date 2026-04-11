# docker

## Run docker

```
docker run <@input>
```

## Execute a shell in a running container

```
(
  sel=$(
    docker ps --format '{{.Names}}\t{{.Image}}\t{{.Status}}' | \
    column -t -s $'\t' | \
    fzf --ansi --header='name | image | status' --prompt='dexec > '
  ) || exit; \
  c=$(awk '{print $1}' <<< "$sel"); \
  docker exec -it "$c" bash 2>/dev/null || docker exec -it "$c" sh
)
```
