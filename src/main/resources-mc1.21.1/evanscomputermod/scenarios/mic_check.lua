-- Record half a second from the default microphone and report its level.
local mic = audio.open_mic()
mic:set_format(16000)
local pcm = mic:record(0.5)
print("LUAMIC", mic.device, #pcm, audio.peak_db(pcm))
mic:close()
