from locust import HttpUser, task

class WebsiteUser(HttpUser):
    @task
    def f100(self):
        self.client.get("/file100.html")

    @task
    def f1000(self):
        self.client.get("/file1000.html")

    @task
    def f10000(self):
        self.client.get("/file10000.html")

    @task(2)
    def f100000(self):
        self.client.get("/file100000.html")
        