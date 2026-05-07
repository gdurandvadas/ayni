import express from "express";
function complex(n:number){let o=0;for(let i=0;i<n;i++){if(i%2===0)o+=i;else if(i%3===0)o-=i;else if(i%5===0)o+=i*2;else if(i%7===0)o-=i*2;else o++;}return o;}
const app = express();
app.get('/greet/:name',(req,res)=>res.send(`Hello, ${req.params.name}! ${complex(40)}`));
app.listen(3000);
