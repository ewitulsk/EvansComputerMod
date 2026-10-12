import ui
import net

# The host that serves the sample repository in the app_store scenario.
HOST = "10.0.7.1"
PORT = 8080

app = ui.App("dev.ecm.netcheck")
status = ui.Label("Checking the network...")


def check(_=None):
    # The first connection asks for the network permission.
    try:
        conn = net.tcp_connect(HOST, PORT)
        net.tcp_close(conn)
        status.text = "Network: allowed (reached %s:%d)" % (HOST, PORT)
    except Exception:
        status.text = "Network: denied or unreachable"
    print(status.text)


app.window("Net Check", ui.Column(gap=8, padding=12, children=[
    ui.Label("Net Check", style="title"),
    status,
    ui.Button("Check again", on_click=check),
]))
check()
app.run()
