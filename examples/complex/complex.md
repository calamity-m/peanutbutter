---
name: Multi-line scripts
tags:
  - scripts
  - shell
---

# Scripts

## Write an .env file from variables

```bash
cat << EOF > <@output:?.env>
APP_ENV=<@environment:?development>
APP_PORT=<@port:?3000>
DATABASE_URL=<@database_url>
SECRET_KEY=<@secret_key>
EOF
```

## Create a Dockerfile for serving static files

There must be a `public/` directory in the current working directory.

```bash
cat << EOF > <@dockerfile:?Dockerfile>
FROM nginx:alpine
COPY public /usr/share/nginx/html
EXPOSE 80
EOF
```

## SSH port forward

```bash
ssh -L <@local_port:?8080>:localhost:<@remote_port:?8080> <@user>@<@host> -N
```

## Archive and compress a directory

```bash
tar -czf <@archive:?archive.tar.gz> <@source:?.>
```

## Curl with method, headers, and body

```bash
curl -s -X <@http_method:printf 'GET\nPOST\nPUT\nPATCH\nDELETE'> \
     -H "Content-Type: application/json" \
     -H "<@header_name:?Authorization>: <@header_value>" \
     -d '<@body:?{}>'\
     <@url>
```
