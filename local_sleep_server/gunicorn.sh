#!/bin/bash
gunicorn -w 2 -k eventlet --threads 60 -b localhost:8070 app:app
