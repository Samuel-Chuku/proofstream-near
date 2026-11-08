# The relayer: plain Node, no npm packages (D40). The image holds only src/.
FROM node:22-slim@sha256:5ec23b7c622b00547753bc6ffd78085dd868a68aeb41c8e0a1418f033e13d96d

ENV NODE_ENV=production
WORKDIR /app
# Owned by root and not writable by the runtime user.
COPY src/ ./src/
USER node
EXPOSE 8080
HEALTHCHECK --interval=30s --timeout=5s --retries=3 \
  CMD ["node", "-e", "fetch('http://127.0.0.1:8080/health').then(r => process.exit(r.ok ? 0 : 1), () => process.exit(1))"]
CMD ["node", "src/main.mjs"]
