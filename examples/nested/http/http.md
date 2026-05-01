---
name: HTTP
tags:
  - http
  - curl
  - api
---

# HTTP

## GET request

```bash
curl -s <@url>
```

## GET with bearer token

```bash
curl -s -H "Authorization: Bearer <@token>" <@url>
```

## POST JSON body

```bash
curl -s -X POST \
     -H "Content-Type: application/json" \
     -d '<@body:?{}>' \
     <@url>
```

## POST JSON with bearer token

```bash
curl -s -X POST \
     -H "Authorization: Bearer <@token>" \
     -H "Content-Type: application/json" \
     -d '<@body:?{}>' \
     <@url>
```

## Check HTTP status code only

```bash
curl -s -o /dev/null -w "%{http_code}" <@url>
```

## Download a file

```bash
curl -L -o <@output_file> <@url>
```

## Upload a file (multipart form)

```bash
curl -s -X POST \
     -F "file=@<@file:rg . --files>" \
     <@url>
```
