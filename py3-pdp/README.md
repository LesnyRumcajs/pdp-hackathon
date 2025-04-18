## PDP uploader GUI in Python

### Installation

You need to install [poetry](https://python-poetry.org/docs/#installation) to manage the dependencies. Otherwise, you can use `pip` manually.

```
poetry install
```

### Other dependencies
- You need to have `docker` installed and running.
- You will also need your public key uploaded to the PDP SP. See [here](https://github.com/LesnyRumcajs/pdp?tab=readme-ov-file#creating-a-service-secret) for more information.

### Usage

To run the GUI, you can use the following command (optionally, go to `poetry shell` first):

```
python3 gui.py
```

State for the intermittent steps is  saved in the `state.json` file.

Configuration is automatically taken from `.env` file. You can use the `.env.example` file as a template.
