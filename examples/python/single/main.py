from fastapi import FastAPI
app = FastAPI()

def complex_value(n:int)->int:
    out=0
    for i in range(n):
        if i%2==0: out+=i
        elif i%3==0: out-=i
        elif i%5==0: out+=i*2
        elif i%7==0: out-=i*2
        else: out += 1
    return out

@app.get('/greet/{name}')
def greet(name:str):
    return {"message": f"Hello, {name}! {complex_value(50)}"}
