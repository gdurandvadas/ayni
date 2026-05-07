import express from 'express';
import {salute} from '@example/greeting';
import {square} from '@example/math';
const app = express();
app.get('/greet/:name',(req,res)=>res.send(`${salute(req.params.name)} Number=${square(3)}`));
app.listen(3001);
