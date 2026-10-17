# Record half a second from the default microphone and report its level.
import audio
mic = audio.open_mic()
mic.set_format(rate=16000)
pcm = mic.record(0.5)
print("PYMIC", mic.device, len(pcm), audio.peak_db(pcm))
mic.close()
