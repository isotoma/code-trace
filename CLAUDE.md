This project will be a rust re-implementation of the python script in `docs/langfuse_hook.py`.

This web page explains the usage: https://langfuse.com/integrations/other/claude-code

The reason to reimplement in rust is:

1. I want a really simple way to install this for our teams of developers
2. I want to avoid python environment messing about
3. Each turn takes 0.5 - 1.0s to send the trace, which will be annoying soon. I want to look at ways of speeding this up as much as possible.

In addition there is a bug in the current python script where tags are not being successfully sent. It is not clear why but it smells like a bug in the new langfuse python sdk v4.
