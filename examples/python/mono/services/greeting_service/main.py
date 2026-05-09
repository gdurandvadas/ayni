from fastapi import FastAPI
from greeting_lib import salute
from math_lib import square

app = FastAPI()

@app.get('/greet/{name}')
def greet(name:str):
    return {"message": f"{salute(name)} Number={square(3)}"}
