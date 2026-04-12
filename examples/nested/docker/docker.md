# docker

You're free to structure things however you want. This example is more contrived, but we have
this docker.md under "docker", and split compose/images into their own snippet files.


## Run docker

```bash
docker run <@input>
```

## Execute a shell in a running container

This is an example of a munted shell snippet. You're better off
making this a permanent shell script, and just snippeting the execution of it. 
Then again, this works :).

```bash
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
