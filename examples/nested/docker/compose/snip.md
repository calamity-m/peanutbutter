---
name: Docker Compose
tags:
  - docker
  - compose
---

# Docker Compose

## Start services in the background

```bash
docker compose up -d
```

## Stop and remove containers

```bash
docker compose down
```

## View logs for a service

```bash
docker compose logs -f <@service>
```

## Rebuild and restart a service

```bash
docker compose up -d --build <@service>
```

## Run a one-off command in a service container

```bash
docker compose exec <@service> <@command:?bash>
```

## Start with a specific compose file

```bash
docker compose -f <@compose_file:rg . --files -g "*.yaml"> up -d
```
