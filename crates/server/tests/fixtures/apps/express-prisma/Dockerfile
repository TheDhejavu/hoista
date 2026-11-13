FROM node:24-alpine
WORKDIR /app
COPY package*.json ./
RUN npm ci --ignore-scripts
COPY prisma ./prisma
RUN DATABASE_URL=postgresql://localhost/fixture npx --no-install prisma generate
COPY server.js ./server.js
USER node
EXPOSE 8080
CMD ["node", "server.js"]
