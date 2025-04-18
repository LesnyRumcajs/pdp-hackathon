import tkinter as tk
from tkinter import filedialog, scrolledtext
from PIL import Image, ImageTk
import subprocess
import re
import os
import json
import time
import zmq
import threading
from dotenv import load_dotenv

# Load environment variables
load_dotenv()

# Config from .env
SERVICE_URL = os.getenv("SERVICE_URL")
SERVICE_NAME = os.getenv("SERVICE_NAME")
RECORDKEEPER = os.getenv("RECORDKEEPER")
PDP_DATA_DIR = os.getenv("PDP_DATA_DIR", "pdp-data")
STATE_FILE = os.getenv("STATE_FILE", "state.json")

PDPD = "docker run -it --mount type=bind,src=$PWD,dst=/data ghcr.io/lesnyrumcajs/pdptool:edge"

class PDPGui:
    def __init__(self, root):
        self.root = root
        self.root.title("PDP Uploader")
        self.root.geometry("800x700")
        self.bg_color = "#1e1e1e"
        self.fg_color = "#d4d4d4"
        self.accent_color = "#007acc"
        root.configure(bg=self.bg_color)

        self.zmq_context = zmq.Context()
        self.zmq_socket = self.zmq_context.socket(zmq.REQ)
        self.zmq_socket.connect("tcp://localhost:5555")

        self.file_path = tk.StringVar()
        self.file_id = ""
        self.tx_hash = ""
        self.proofset_id = ""

        self.create_widgets()
        self.load_state()

    def create_widgets(self):
        tk.Button(self.root, text="Upload & Process", command=self.start_process_thread,
                  bg=self.accent_color, fg="white", activebackground="#005f9e",
                  relief="flat", padx=10, pady=5).pack(pady=10)

        tk.Entry(self.root, textvariable=self.file_path, width=100,
                 bg="#2d2d2d", fg=self.fg_color, insertbackground=self.fg_color).pack(pady=5)

        self.image_label = tk.Label(self.root, bg=self.bg_color)
        self.image_label.pack(pady=10)

        self.log_area = scrolledtext.ScrolledText(self.root, height=15,
                                                  bg="#1e1e1e", fg=self.fg_color,
                                                  insertbackground=self.fg_color)
        self.log_area.pack(fill="both", expand=True, padx=10, pady=10)

        self.status_label = tk.Label(self.root, text="Ready", anchor="w",
                                     fg="gray", bg=self.bg_color)
        self.status_label.pack(fill="x", padx=10, pady=2)

    def preview_image(self, path):
        try:
            img = Image.open(path)
            img.thumbnail((300, 300))
            img_tk = ImageTk.PhotoImage(img)
            self.image_label.configure(image=img_tk)
            self.image_label.image = img_tk
        except Exception:
            self.image_label.configure(image=None)
            self.image_label.image = None

    def load_state(self):
        if os.path.exists(STATE_FILE):
            with open(STATE_FILE, 'r') as f:
                state = json.load(f)
                self.file_id = state.get("file_id", "")
                self.tx_hash = state.get("tx_hash", "")
                self.proofset_id = state.get("proofset_id", "")

    def save_state(self):
        with open(STATE_FILE, 'w') as f:
            json.dump({"file_id": self.file_id, "tx_hash": self.tx_hash, "proofset_id": self.proofset_id}, f)

    def run_cmd(self, cmd):
        self.log(f"> {cmd}")
        result = subprocess.run(["sh", "-c", cmd], capture_output=True, text=True)
        self.log(result.stdout)
        if result.stderr:
            self.log(result.stderr)
        return result.stdout + result.stderr

    def send_zmq_message(self, stage, data):
        msg = json.dumps({"stage": stage, "data": data})
        self.zmq_socket.send_string(msg)
        self.zmq_socket.recv()
        self.log(f"ZMQ: {msg}")

    def start_process_thread(self):
        threading.Thread(target=self.process, daemon=True).start()

    def process(self):
        filepath = filedialog.askopenfilename()
        if not filepath:
            return

        self.file_path.set(filepath)
        filename = os.path.basename(filepath)
        self.preview_image(filepath)

        os.makedirs(PDP_DATA_DIR, exist_ok=True)
        dst = f"{PDP_DATA_DIR}/{filename}"
        subprocess.run(["cp", filepath, dst])

        self.set_status("Uploading file...")
        output = self.run_cmd(f"{PDPD} upload-file --service-url {SERVICE_URL} --service-name {SERVICE_NAME} /data/{filename}")
        match = re.search(r"(baga[a-zA-Z0-9]+:[a-zA-Z0-9]+)", output)
        if not match:
            self.set_status("Upload failed", "red")
            return

        self.file_id = match.group(1)
        self.send_zmq_message("Uploaded", {"file": filename, "file_id": self.file_id})
        self.set_status("Uploaded successfully")
        self.save_state()

        if not self.proofset_id:
            self.set_status("Creating proof set...")
            out = self.run_cmd(f"{PDPD} create-proof-set --service-url {SERVICE_URL} --service-name {SERVICE_NAME} --recordkeeper {RECORDKEEPER}")
            tx_match = re.search(r"0x[a-fA-F0-9]{64}", out)
            if not tx_match:
                self.set_status("Proof set creation failed", "red")
                return

            self.tx_hash = tx_match.group(0)
            self.save_state()

            self.set_status("Waiting for proof set confirmation...")
            while True:
                time.sleep(5)
                check = self.run_cmd(f"{PDPD} get-proof-set-create-status --service-url {SERVICE_URL} --service-name {SERVICE_NAME} --tx-hash {self.tx_hash}")
                proofset_match = re.search(r"ProofSet ID:\s+(\d+)", check)
                if proofset_match:
                    self.proofset_id = proofset_match.group(1)
                    self.save_state()
                    self.set_status(f"Proof set ready: {self.proofset_id}")
                    break

        self.set_status("Adding roots...")
        self.run_cmd(f"{PDPD} add-roots --service-url {SERVICE_URL} --service-name {SERVICE_NAME} --proof-set-id={self.proofset_id} --root {self.file_id}")
        self.set_status("Roots added successfully")
        self.send_zmq_message("RootsAdded", {"file": filename, "file_id": self.file_id, "proofset_id": self.proofset_id})

    def log(self, message):
        self.log_area.insert("end", message + "\n")
        self.log_area.see("end")

    def set_status(self, message, color="gray"):
        self.status_label.config(text=message, fg=color)


if __name__ == "__main__":
    root = tk.Tk()
    app = PDPGui(root)
    root.mainloop()
