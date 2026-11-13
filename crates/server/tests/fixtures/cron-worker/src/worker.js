import { message } from './message.js';
export default {
  async scheduled(controller, env, ctx) {
    ctx.waitUntil(Promise.resolve().then(() => console.log(message(controller.cron))));
  },
};
