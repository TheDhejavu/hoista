import express from 'express';
import { PrismaClient } from '@prisma/client';
const database = new PrismaClient();
const app = express();
app.get('/health', (_request, response) => response.json({ status: 'ok' }));
app.get('/items', async (_request, response) => response.json(await database.item.findMany()));
app.listen(Number(process.env['PORT'] || 8080), '0.0.0.0');
