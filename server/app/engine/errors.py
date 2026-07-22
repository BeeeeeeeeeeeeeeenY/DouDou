class ConfigError(Exception):
    """配置缺失导致无法执行（面向用户的中文提示）。"""

    def __init__(self, message: str):
        self.message = message
        super().__init__(message)
