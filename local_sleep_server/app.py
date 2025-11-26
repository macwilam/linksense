#!/usr/bin/env python3

import random
import time

# import gevent
from bottle import Bottle, run, template

app = Bottle()


@app.route("/")
def root():
    time.sleep(0.2)  # 200ms sleep
    return "OK"


@app.route("/sleep/<sleep>/")
def sleep_route(sleep=200):
    try:
        time.sleep(int(sleep) / 1000.0)  # Convert milliseconds to seconds
    except:
        time.sleep(0.2)
    return "OK"


@app.route("/random_a_b")
def random_template():
    # Return template a 90% of the time, template b 10% of the time
    if random.random() < 0.9:
        return template("a.html")
    else:
        return template("b.html")


if __name__ == "__main__":
    run(app, host="localhost", port=8071, debug=True)
