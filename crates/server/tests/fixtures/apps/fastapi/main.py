import os
from fastapi import FastAPI

app = FastAPI()
label = os.getenv("APP_LABEL", "Hoista FastAPI fixture")

@app.get("/health")
def health():
    return {"status": "ok", "label": label}
